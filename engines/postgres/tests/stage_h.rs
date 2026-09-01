use dol::prelude::*;
use dol_engine::{ArtifactCacheability, Engine};
use dol_postgres::{
    ColumnMapping, PostgresCatalog, PostgresEngine, PostgresRuntimeConfig, TableMapping,
};

#[derive(Clone, Debug, PartialEq, dol::Model)]
#[dol(key = "stage-h/account", name = "StageHAccount")]
struct Account {
    #[dol(identity)]
    id: u64,
    active: bool,
    balance: i64,
    #[dol(optional)]
    nickname: Option<String>,
}

#[derive(Clone, Debug, PartialEq, dol::Model)]
#[dol(key = "stage-h/account", name = "StageHAccountV2")]
struct AccountV2 {
    #[dol(identity)]
    id: u64,
    active: bool,
    balance: i64,
    #[dol(optional)]
    nickname: Option<String>,
    revision: u64,
}

#[derive(Clone, Debug, PartialEq, dol::Model)]
#[dol(key = "stage-h/comparable", name = "StageHComparable")]
struct Comparable {
    #[dol(identity)]
    id: u64,
    score: f64,
    label: String,
}

fn catalog() -> PostgresCatalog {
    let model = Account::model_def().unwrap();
    let mapping = TableMapping::new("public", "accounts")
        .field("id", ColumnMapping::required("id"))
        .field("active", ColumnMapping::required("active"))
        .field("balance", ColumnMapping::required("balance"))
        .field(
            "nickname",
            ColumnMapping::optional("nickname", "nickname_present"),
        );
    let mut catalog = PostgresCatalog::new();
    catalog.register(model, mapping).unwrap();
    catalog
}

fn comparable_catalog() -> PostgresCatalog {
    let model = Comparable::model_def().unwrap();
    let mapping = TableMapping::new("public", "comparables")
        .field("id", ColumnMapping::required("id"))
        .field("score", ColumnMapping::required("score"))
        .field("label", ColumnMapping::required("label"));
    let mut catalog = PostgresCatalog::new();
    catalog.register(model, mapping).unwrap();
    catalog
}

#[test]
fn postgres_slice_one_does_not_overclaim_runtime_capabilities() {
    let engine = PostgresEngine::new(catalog());
    assert_eq!(engine.info().kind, "postgres");
    let capabilities = engine.capabilities();
    assert!(!capabilities.pipeline.source.is_supported());
    assert!(!capabilities.pipeline.filter.is_supported());
    assert!(!capabilities.pipeline.project.is_supported());
    assert!(!capabilities.writes.insert.is_supported());
    assert!(!capabilities.transaction.transactions.is_supported());
}

#[test]
fn optional_fields_require_a_presence_column() {
    let model = Account::model_def().unwrap();
    let mapping = TableMapping::new("public", "accounts")
        .field("id", ColumnMapping::required("id"))
        .field("active", ColumnMapping::required("active"))
        .field("balance", ColumnMapping::required("balance"))
        .field("nickname", ColumnMapping::required("nickname"));
    let mut catalog = PostgresCatalog::new();
    let error = catalog.register(model, mapping).unwrap_err();
    assert_eq!(error.code(), "POSTGRES-MAP-004");
}

#[test]
fn validated_mapping_is_safe_for_repeated_exact_model_lookup() {
    let catalog = catalog();
    let model = Account::model_def().unwrap();
    let first = catalog.table(model).unwrap();
    let second = catalog.table(model).unwrap();
    assert!(std::ptr::eq(first, second));
}

#[test]
fn parameterized_filter_project_and_slice_compile_without_value_interpolation() {
    let engine = PostgresEngine::new(catalog());
    let minimum = Parameter::<i64>::new("minimum");
    let parameters = Parameters::new().with(&minimum, 25_i64);
    let pipeline = Pipeline::<Account>::from_model()
        .filter(Account::active.eq(true).and(Account::balance.gt(&minimum)))
        .select((Account::id, Account::nickname))
        .limit(5);
    let plan = pipeline.logical_plan().unwrap();
    let compiled = engine.compile(&plan, &parameters).unwrap();

    assert_eq!(compiled.cacheability(), ArtifactCacheability::RequestScoped);

    assert!(compiled.sql().contains("FROM \"public\".\"accounts\""));
    assert!(compiled.sql().contains("$1::smallint"));
    assert!(compiled.sql().contains("$3::smallint"));
    let typed_order = (compiled.sql().contains("$2::boolean")
        && compiled.sql().contains("$4::bigint"))
        || (compiled.sql().contains("$2::bigint") && compiled.sql().contains("$4::boolean"));
    assert!(typed_order);
    assert!(!compiled.sql().contains("25"));
    assert!(compiled.sql().contains("LIMIT 5"));
    assert_eq!(compiled.binds().len(), 4);
    assert_eq!(compiled.columns().len(), 2);
    assert_eq!(compiled.output(), plan.output());
}

#[test]
fn parameter_state_does_not_change_sql_shape_or_placeholder_layout() {
    let engine = PostgresEngine::new(catalog());
    let minimum = Parameter::<Option<i64>>::new("minimum");
    let pipeline = Pipeline::<Account>::from_model()
        .filter(Account::balance.nullable().eq(&minimum))
        .select(Account::id);
    let plan = pipeline.logical_plan().unwrap();

    let present = engine
        .compile(&plan, &Parameters::new().with(&minimum, Some(25_i64)))
        .unwrap();
    let null = engine
        .compile(&plan, &Parameters::new().with(&minimum, None))
        .unwrap();

    assert_eq!(present.sql(), null.sql());
    assert_eq!(present.binds().len(), null.binds().len());
    assert_eq!(present.columns(), null.columns());
    assert_eq!(present.binds()[0].state_value(), Some(2));
    assert_eq!(null.binds()[0].state_value(), Some(1));
}

#[test]
fn catalog_rejects_conflicting_exact_models_under_one_lineage() {
    let mut catalog = catalog();
    let changed = AccountV2::model_def().unwrap();
    let mapping = TableMapping::new("public", "accounts_v2")
        .field("id", ColumnMapping::required("id"))
        .field("active", ColumnMapping::required("active"))
        .field("balance", ColumnMapping::required("balance"))
        .field(
            "nickname",
            ColumnMapping::optional("nickname", "nickname_present"),
        )
        .field("revision", ColumnMapping::required("revision"));

    let error = catalog.register(changed, mapping).unwrap_err();
    assert_eq!(error.code(), "POSTGRES-MAP-009");
}

#[test]
fn missing_and_null_use_distinct_physical_state() {
    let engine = PostgresEngine::new(catalog());
    let pipeline = Pipeline::<Account>::from_model()
        .filter(Account::nickname.is_missing())
        .select(Account::id);
    let plan = pipeline.logical_plan().unwrap();
    let compiled = engine.compile(&plan, &Parameters::new()).unwrap();

    assert!(compiled.sql().contains("\"nickname_present\""));
    assert!(compiled.sql().contains("THEN 0"));
    assert!(compiled.sql().contains("THEN 1"));
    assert!(compiled.sql().contains("ELSE 2"));
}

#[test]
fn compiler_is_deterministic_for_the_same_plan_and_parameters() {
    let engine = PostgresEngine::new(catalog());
    let minimum = Parameter::<i64>::new("minimum");
    let parameters = Parameters::new().with(&minimum, 10_i64);
    let pipeline = Pipeline::<Account>::from_model()
        .filter(Account::balance.ge(&minimum))
        .select(Account::id)
        .offset(2)
        .limit(3);
    let plan = pipeline.logical_plan().unwrap();

    let first = engine.compile(&plan, &parameters).unwrap();
    let second = engine.compile(&plan, &parameters).unwrap();
    assert_eq!(first.sql(), second.sql());
    assert_eq!(first.binds().len(), second.binds().len());
    assert_eq!(first.columns(), second.columns());
}

#[test]
fn unsupported_semantics_are_rejected_instead_of_approximated() {
    let engine = PostgresEngine::new(catalog());
    let pipeline = Pipeline::<Account>::from_model().select(Account::balance + 1_i64);
    let plan = pipeline.logical_plan().unwrap();
    let error = engine.compile(&plan, &Parameters::new()).unwrap_err();
    assert_eq!(error.code(), "POSTGRES-COMPILE-013");
}

#[test]
fn physical_identifiers_are_quoted_not_interpolated_as_sql_syntax() {
    let model = Account::model_def().unwrap();
    let mapping = TableMapping::new("tenant data", "accounts\";drop table x;--")
        .field("id", ColumnMapping::required("id"))
        .field("active", ColumnMapping::required("active"))
        .field("balance", ColumnMapping::required("balance"))
        .field(
            "nickname",
            ColumnMapping::optional("nickname", "nickname_present"),
        );
    let mut catalog = PostgresCatalog::new();
    catalog.register(model, mapping).unwrap();
    let engine = PostgresEngine::new(catalog);
    let plan = Pipeline::<Account>::from_model().logical_plan().unwrap();
    let compiled = engine.compile(&plan, &Parameters::new()).unwrap();

    assert!(
        compiled
            .sql()
            .contains("\"tenant data\".\"accounts\"\";drop table x;--\"")
    );
}

#[test]
fn physical_identifier_length_is_rejected_before_sql_generation() {
    let model = Account::model_def().unwrap();
    let mapping = TableMapping::new("x".repeat(64), "accounts")
        .field("id", ColumnMapping::required("id"))
        .field("active", ColumnMapping::required("active"))
        .field("balance", ColumnMapping::required("balance"))
        .field(
            "nickname",
            ColumnMapping::optional("nickname", "nickname_present"),
        );
    let mut catalog = PostgresCatalog::new();
    let error = catalog.register(model, mapping).unwrap_err();
    assert_eq!(error.code(), "POSTGRES-MAP-010");
}

#[test]
fn compiler_compensates_for_postgres_float_and_text_comparison_semantics() {
    let engine = PostgresEngine::new(comparable_catalog());
    let pipeline = Pipeline::<Comparable>::from_model()
        .filter(
            Comparable::score
                .eq(f64::NAN)
                .and(Comparable::label.lt(String::from("z"))),
        )
        .select(Comparable::id);
    let plan = pipeline.logical_plan().unwrap();
    let compiled = engine.compile(&plan, &Parameters::new()).unwrap();

    assert!(compiled.sql().contains("'NaN'::double precision"));
    assert!(compiled.sql().contains("COLLATE \"C\""));
}

#[test]
fn slice_values_outside_postgres_bigint_range_are_rejected_exactly() {
    let engine = PostgresEngine::new(catalog());
    let pipeline = Pipeline::<Account>::from_model().offset(u64::MAX);
    let plan = pipeline.logical_plan().unwrap();
    let error = engine.compile(&plan, &Parameters::new()).unwrap_err();
    assert_eq!(error.code(), "POSTGRES-COMPILE-018");
}

#[test]
fn runtime_requires_explicit_tls_and_redacts_passwords() {
    let insecure =
        PostgresRuntimeConfig::parse("host=localhost user=dol dbname=dol sslmode=disable")
            .unwrap_err();
    assert_eq!(insecure.code(), "POSTGRES-CONNECT-002");

    let runtime = PostgresRuntimeConfig::parse(
        "host=localhost user=dol password=super-secret dbname=dol sslmode=require",
    )
    .unwrap();
    let debug = format!("{runtime:?}");
    assert!(debug.contains("dol"));
    assert!(!debug.contains("super-secret"));
    assert!(debug.contains("custom_trusted_ca: false"));

    let invalid_ca = runtime
        .with_trusted_ca_pem(b"not a PEM certificate")
        .unwrap_err();
    assert_eq!(invalid_ca.code(), "POSTGRES-CONNECT-005");
}

#[test]
fn runtime_advertises_only_the_exact_slice_two_read_surface() {
    let runtime =
        PostgresRuntimeConfig::parse("host=localhost user=dol dbname=dol sslmode=require").unwrap();
    let engine = PostgresEngine::with_runtime(catalog(), runtime);
    let capabilities = engine.capabilities();

    assert!(capabilities.pipeline.source.is_native());
    assert!(capabilities.pipeline.filter.is_supported());
    assert!(capabilities.pipeline.project.is_supported());
    assert!(capabilities.pipeline.slice.is_native());
    assert!(!capabilities.pipeline.aggregate.is_supported());
    assert!(!capabilities.pipeline.sort.is_supported());
    assert!(!capabilities.pipeline.join.is_supported());
    assert!(!capabilities.writes.insert.is_supported());
    assert!(!capabilities.transaction.transactions.is_supported());
}

#[test]
fn runtime_explain_refines_capabilities_for_concrete_expressions() {
    let runtime =
        PostgresRuntimeConfig::parse("host=localhost user=dol dbname=dol sslmode=require").unwrap();
    let engine = PostgresEngine::with_runtime(catalog(), runtime);
    let pipeline = Pipeline::<Account>::from_model().select(Account::balance + 1_i64);
    let error = engine
        .explain(&pipeline, dol_engine::PlacementPolicy::RemoteOnly)
        .unwrap_err();
    assert_eq!(error.code(), "ENGINE-PLACEMENT-001");
}

#[test]
fn unsigned_numeric_binds_and_outputs_use_exact_text_transport() {
    let engine = PostgresEngine::new(catalog());
    let wanted = Parameter::<u64>::new("wanted");
    let pipeline = Pipeline::<Account>::from_model()
        .filter(Account::id.eq(&wanted))
        .select(Account::id);
    let plan = pipeline.logical_plan().unwrap();
    let compiled = engine
        .compile(&plan, &Parameters::new().with(&wanted, u64::MAX))
        .unwrap();

    assert!(compiled.sql().contains("($2::text)::numeric"));
    assert!(compiled.sql().contains("::text AS"));
    assert!(compiled.sql().contains("::smallint AS"));
    assert!(!compiled.sql().contains(&u64::MAX.to_string()));
    assert_eq!(compiled.binds().len(), 2);
}
