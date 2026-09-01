use dol::prelude::*;
use dol_engine::{ArtifactCacheability, Engine};
use dol_mongodb::{
    CollectionMapping, FieldMapping, MongodbCatalog, MongodbEngine, MongodbRuntimeConfig,
};

#[derive(Clone, Debug, PartialEq, dol::Model)]
#[dol(key = "stage-i/account", name = "StageIAccount")]
struct Account {
    #[dol(identity)]
    id: u64,
    active: bool,
    balance: i64,
    #[dol(optional)]
    nickname: Option<String>,
}

#[derive(Clone, Debug, PartialEq, dol::Model)]
#[dol(key = "stage-i/account", name = "StageIAccountV2")]
struct AccountV2 {
    #[dol(identity)]
    id: u64,
    active: bool,
    balance: i64,
    #[dol(optional)]
    nickname: Option<String>,
    revision: u64,
}

fn catalog() -> MongodbCatalog {
    let mapping = CollectionMapping::new("dol_stage_i", "accounts")
        .field("id", FieldMapping::new("id"))
        .field("active", FieldMapping::new("active"))
        .field("balance", FieldMapping::new("balance"))
        .field("nickname", FieldMapping::new("nickname"));
    let mut catalog = MongodbCatalog::new();
    catalog
        .register(Account::model_def().unwrap(), mapping)
        .unwrap();
    catalog
}

#[test]
fn offline_engine_does_not_advertise_runtime_support() {
    let engine = MongodbEngine::new(catalog());
    assert_eq!(engine.info().kind, "mongodb");
    assert!(!engine.capabilities().pipeline.source.is_supported());
    assert!(!engine.capabilities().writes.insert.is_supported());
}

#[test]
fn exact_filter_project_slice_compilation_is_deterministic() {
    let engine = MongodbEngine::new(catalog());
    let minimum = Parameter::<i64>::new("minimum");
    let parameters = Parameters::new().with(&minimum, 25_i64);
    let pipeline = Pipeline::<Account>::from_model()
        .filter(Account::active.eq(true).and(Account::balance.gt(&minimum)))
        .select((Account::id, Account::nickname))
        .offset(2)
        .limit(5);
    let plan = pipeline.logical_plan().unwrap();

    let first = engine.compile(&plan, &parameters).unwrap();
    let second = engine.compile(&plan, &parameters).unwrap();
    assert_eq!(first.database(), "dol_stage_i");
    assert_eq!(first.collection(), "accounts");
    assert_eq!(first.pipeline(), second.pipeline());
    assert_eq!(first.fields(), second.fields());
    assert_eq!(first.output(), plan.output());
    assert_eq!(first.fields().len(), 2);
    assert_eq!(first.cacheability(), ArtifactCacheability::RequestScoped);

    let rendered = format!("{:?}", first.pipeline());
    assert!(rendered.contains("$match"));
    assert!(rendered.contains("$skip"));
    assert!(rendered.contains("$limit"));
    assert!(rendered.contains("$literal"));
}

#[test]
fn source_preflight_and_projection_keep_missing_distinct_from_null() {
    let engine = MongodbEngine::new(catalog());
    let pipeline = Pipeline::<Account>::from_model()
        .filter(Account::nickname.is_missing())
        .select(Account::id);
    let plan = pipeline.logical_plan().unwrap();
    let compiled = engine.compile(&plan, &Parameters::new()).unwrap();

    let rendered = format!("{:?}", compiled.pipeline());
    assert!(rendered.contains("$type"));
    assert!(rendered.contains("missing"));
    assert!(rendered.contains("null"));
    assert!(rendered.contains("__dol_"));
    assert_eq!(compiled.validation_pipeline().len(), 3);
}

#[test]
fn unsupported_semantics_are_rejected_not_approximated() {
    let engine = MongodbEngine::new(catalog());
    let pipeline = Pipeline::<Account>::from_model().select(Account::balance + 1_i64);
    let error = engine
        .compile(&pipeline.logical_plan().unwrap(), &Parameters::new())
        .unwrap_err();
    assert_eq!(error.code(), "MONGODB-COMPILE-011");
}

#[test]
fn catalog_rejects_reserved_fields_and_stale_exact_models() {
    let model = Account::model_def().unwrap();
    let invalid = CollectionMapping::new("dol_stage_i", "accounts")
        .field("id", FieldMapping::new("__dol_id"))
        .field("active", FieldMapping::new("active"))
        .field("balance", FieldMapping::new("balance"))
        .field("nickname", FieldMapping::new("nickname"));
    let error = MongodbCatalog::new().register(model, invalid).unwrap_err();
    assert_eq!(error.code(), "MONGODB-MAP-009");

    let mut catalog = catalog();
    let changed = AccountV2::model_def().unwrap();
    let mapping = CollectionMapping::new("dol_stage_i", "accounts_v2")
        .field("id", FieldMapping::new("id"))
        .field("active", FieldMapping::new("active"))
        .field("balance", FieldMapping::new("balance"))
        .field("nickname", FieldMapping::new("nickname"))
        .field("revision", FieldMapping::new("revision"));
    let error = catalog.register(changed, mapping).unwrap_err();
    assert_eq!(error.code(), "MONGODB-MAP-005");
}

#[test]
fn validated_mapping_is_safe_for_repeated_exact_model_lookup() {
    let catalog = catalog();
    let model = Account::model_def().unwrap();
    let first = catalog.collection(model).unwrap();
    let second = catalog.collection(model).unwrap();
    assert!(std::ptr::eq(first, second));
}

#[test]
fn runtime_configuration_redacts_credentials_and_claims_only_read_slice() {
    let runtime = MongodbRuntimeConfig::new("mongodb://user:super-secret@localhost:27017")
        .application_name("dol-stage-i");
    let debug = format!("{runtime:?}");
    assert!(!debug.contains("super-secret"));
    assert!(debug.contains("dol-stage-i"));

    let engine = MongodbEngine::with_runtime(catalog(), runtime);
    let capabilities = engine.capabilities();
    assert!(capabilities.pipeline.source.is_supported());
    assert!(capabilities.pipeline.filter.is_supported());
    assert!(capabilities.pipeline.project.is_supported());
    assert!(capabilities.pipeline.slice.is_native());
    assert!(!capabilities.pipeline.aggregate.is_supported());
    assert!(!capabilities.pipeline.join.is_supported());
    assert!(!capabilities.writes.insert.is_supported());
}

#[test]
fn runtime_explain_refines_capabilities_for_concrete_expressions() {
    let engine = MongodbEngine::with_runtime(
        catalog(),
        MongodbRuntimeConfig::new("mongodb://localhost:27017"),
    );
    let pipeline = Pipeline::<Account>::from_model().select(Account::balance + 1_i64);
    let error = engine
        .explain(&pipeline, dol_engine::PlacementPolicy::RemoteOnly)
        .unwrap_err();
    assert_eq!(error.code(), "ENGINE-PLACEMENT-001");
}

#[test]
fn slice_outside_mongodb_i64_range_is_rejected() {
    let engine = MongodbEngine::new(catalog());
    let pipeline = Pipeline::<Account>::from_model().offset(u64::MAX);
    let error = engine
        .compile(&pipeline.logical_plan().unwrap(), &Parameters::new())
        .unwrap_err();
    assert_eq!(error.code(), "MONGODB-COMPILE-017");
}
