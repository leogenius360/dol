//! User-attested historical equivalents and decomposed modern workloads.

use std::hint::black_box;
use std::mem::size_of;

use dol::prelude::*;
use dol_bench::{Config, Runner, Suite, help};
use dol_core::expr::EvalContext;
use dol_core::model::{ModelBuilder, ModelDef};
use dol_engine::{
    Engine, ExecutionOptions, ExplainPlan, PlacementPolicy, analyze_engine_placement,
    collect_stream,
};
use dol_memory::MemoryEngine;
use dol_migrate::{
    CatalogEntity, CatalogField, CatalogIndex, CatalogRevision, CatalogScope, CatalogSnapshot,
    DefaultMigrationPlanner, MigrationIntent, MigrationPlanner,
};
use dol_ml::{
    Distance, ExactSearchRequest, SearchLimits, Vector, VectorCandidate, VectorId, exact_search,
};
use dol_mongodb::{CollectionMapping, FieldMapping, MongodbCatalog, MongodbEngine};
use dol_postgres::{
    ColumnMapping, PostgresCatalog, PostgresEngine, PostgresRuntimeConfig, TableMapping,
};
use dol_wire::{DecodeLimits, decode_type_def, encode_type_def};

#[derive(Clone, Debug, PartialEq, dol::Model)]
#[dol(key = "bench/account", name = "BenchAccount")]
struct BenchAccount {
    #[dol(identity)]
    id: u64,
    active: bool,
    balance: i64,
    #[dol(optional)]
    nickname: Option<String>,
}

#[derive(Clone, Debug, PartialEq, dol::Model)]
#[dol(key = "historical/benchmark_rows", name = "benchmark_rows")]
struct BenchmarkRow {
    #[dol(identity)]
    id: u64,
    value: i32,
    active: bool,
    label: String,
}

fn main() {
    if std::env::args().any(|argument| argument == "--help" || argument == "-h") {
        println!("{}", help());
        return;
    }
    let config = Config::parse_env().unwrap_or_else(|error| {
        eprintln!("{error}");
        std::process::exit(2);
    });
    let mut runner = Runner::new(config);
    if runner.suite_enabled(Suite::Historical) {
        historical_benchmarks(&mut runner);
        representation_measurements(&mut runner);
    }
    if runner.suite_enabled(Suite::Modern) {
        modern_benchmarks(&mut runner);
    }
    runner.finish().expect("benchmark report remains writable");
}

fn historical_benchmarks(runner: &mut Runner) {
    runner.measure(
        Suite::Historical,
        "Expression construction (historical fixture, current-API equivalent)",
        100_000,
        historical_expression,
    );

    let model = BenchmarkRow::model_def().expect("historical benchmark model remains valid");
    let prepared = historical_expression()
        .prepare_for(model)
        .expect("historical expression remains valid");
    let row = historical_row();
    let context = EvalContext::single(&row);
    assert_eq!(
        prepared
            .evaluate_truth(&context)
            .expect("historical truth result remains valid"),
        Truth::True
    );
    runner.measure(
        Suite::Historical,
        "Prepared expression evaluation (historical fixture, current-API equivalent)",
        1_000_000,
        || {
            prepared
                .evaluate_truth(black_box(&context))
                .expect("historical evaluation remains valid")
        },
    );

    let expression = historical_expression();
    let pipeline = historical_pipeline();
    runner.measure(
        Suite::Historical,
        "Expression/pipeline clone plus fingerprints (current-API equivalent)",
        100_000,
        || {
            let expression = black_box(&expression).clone();
            let pipeline = black_box(&pipeline).clone();
            (
                expression
                    .fingerprint()
                    .expect("historical expression remains valid"),
                pipeline
                    .fingerprint()
                    .expect("historical pipeline remains valid"),
            )
        },
    );

    let historical_plan = pipeline
        .logical_plan()
        .expect("historical pipeline lowering remains valid");
    let historical_postgres = postgres_runtime_engine(historical_postgres_catalog());
    let capability = historical_postgres
        .explain_plan(&historical_plan, PlacementPolicy::RemoteOnly)
        .map_or_else(
            |diagnostic| format!("unsupported by the current exact boundary: {diagnostic}"),
            |_| "unexpectedly supported; review the exact-compiler boundary".into(),
        );
    runner.note(
        Suite::Historical,
        "PostgreSQL capability analysis and planning",
        capability,
    );

    runner.measure(
        Suite::Historical,
        "Runtime model definition (ModelBuilder::freeze current-API equivalent)",
        10_000,
        build_historical_runtime_model,
    );
}

fn modern_benchmarks(runner: &mut Runner) {
    runner.measure(
        Suite::Modern,
        "Expression construction (modern semantic workload)",
        100_000,
        build_expression,
    );

    let model = BenchAccount::model_def().expect("benchmark model remains valid");
    let expression = build_expression();
    runner.measure(Suite::Modern, "Expression preparation", 100_000, || {
        black_box(&expression)
            .prepare_for(model)
            .expect("benchmark expression preparation remains valid")
    });

    let row = benchmark_account(42);
    runner.measure(
        Suite::Modern,
        "Direct prepare plus evaluate",
        100_000,
        || {
            black_box(&expression)
                .evaluate_truth(black_box(&row))
                .expect("direct benchmark evaluation remains valid")
        },
    );
    let prepared = expression
        .prepare_for(model)
        .expect("benchmark expression remains valid");
    let context = EvalContext::single(&row);
    runner.measure(
        Suite::Modern,
        "Prepared expression evaluation",
        1_000_000,
        || {
            prepared
                .evaluate_truth(black_box(&context))
                .expect("prepared benchmark evaluation remains valid")
        },
    );
    runner.measure(
        Suite::Modern,
        "Runtime model definition (modern semantic workload)",
        10_000,
        build_runtime_model,
    );

    let pipeline = benchmark_pipeline();
    runner.measure(
        Suite::Modern,
        "Expression/pipeline clone only",
        100_000,
        || (black_box(&expression).clone(), black_box(&pipeline).clone()),
    );
    runner.measure(
        Suite::Modern,
        "Cold expression fingerprint",
        100_000,
        || {
            build_expression()
                .fingerprint()
                .expect("cold expression fingerprint remains valid")
        },
    );
    expression
        .fingerprint()
        .expect("warm expression fingerprint remains valid");
    runner.measure(
        Suite::Modern,
        "Warm expression fingerprint",
        100_000,
        || {
            black_box(&expression)
                .fingerprint()
                .expect("warm expression fingerprint remains valid")
        },
    );
    runner.measure(Suite::Modern, "Cold pipeline lowering", 10_000, || {
        benchmark_pipeline()
            .prepared_plan()
            .expect("cold pipeline lowering remains valid")
    });
    pipeline
        .prepared_plan()
        .expect("warm pipeline lowering remains valid");
    runner.measure(Suite::Modern, "Warm pipeline lowering", 100_000, || {
        black_box(&pipeline)
            .prepared_plan()
            .expect("warm pipeline lowering remains valid")
    });
    runner.measure(Suite::Modern, "Cold pipeline fingerprint", 10_000, || {
        benchmark_pipeline()
            .fingerprint()
            .expect("cold pipeline fingerprint remains valid")
    });
    runner.measure(Suite::Modern, "Warm pipeline fingerprint", 100_000, || {
        black_box(&pipeline)
            .fingerprint()
            .expect("warm pipeline fingerprint remains valid")
    });
    runner.measure(
        Suite::Modern,
        "Expression/pipeline clone plus fingerprints (modern semantic workload)",
        100_000,
        || {
            let expression = black_box(&expression).clone();
            let pipeline = black_box(&pipeline).clone();
            (
                expression
                    .fingerprint()
                    .expect("benchmark expression remains valid"),
                pipeline
                    .fingerprint()
                    .expect("benchmark pipeline remains valid"),
            )
        },
    );

    let plan = pipeline
        .logical_plan()
        .expect("benchmark pipeline remains valid");
    let parameters = Parameters::new();
    let postgres = postgres_runtime_engine(postgres_catalog());
    runner.measure(
        Suite::Modern,
        "PostgreSQL capability analysis and planning (modern semantic workload)",
        10_000,
        || {
            postgres
                .explain(black_box(&pipeline), PlacementPolicy::RemoteOnly)
                .expect("benchmark explain plan remains valid")
        },
    );
    runner.measure(Suite::Modern, "PostgreSQL placement", 10_000, || {
        analyze_engine_placement(
            black_box(&plan),
            black_box(&postgres),
            PlacementPolicy::RemoteOnly,
        )
        .expect("benchmark placement remains valid")
    });
    let placement = analyze_engine_placement(&plan, &postgres, PlacementPolicy::RemoteOnly)
        .expect("benchmark placement remains valid");
    runner.measure(
        Suite::Modern,
        "PostgreSQL explain construction",
        10_000,
        || ExplainPlan::new(postgres.info(), black_box(&plan), black_box(&placement)),
    );
    runner.measure(Suite::Modern, "PostgreSQL SQL compilation", 10_000, || {
        postgres
            .compile(black_box(&plan), black_box(&parameters))
            .expect("benchmark PostgreSQL compilation remains valid")
    });
    runner.measure(
        Suite::Modern,
        "PostgreSQL supported placement plus compilation",
        10_000,
        || {
            analyze_engine_placement(
                black_box(&plan),
                black_box(&postgres),
                PlacementPolicy::RemoteOnly,
            )
            .expect("benchmark placement remains valid");
            postgres
                .compile(black_box(&plan), black_box(&parameters))
                .expect("benchmark PostgreSQL compilation remains valid")
        },
    );

    let mongodb = MongodbEngine::new(mongodb_catalog());
    runner.measure(Suite::Modern, "MongoDB offline compilation", 10_000, || {
        mongodb
            .compile(black_box(&plan), black_box(&parameters))
            .expect("benchmark MongoDB compilation remains valid")
    });

    let wire_type = <Vec<Option<String>> as DataType>::type_def();
    let encoded = encode_type_def(&wire_type).expect("benchmark wire type remains valid");
    let wire_name = format!("Wire TypeDef decode ({} byte frame)", encoded.len());
    runner.measure(Suite::Modern, wire_name, 100_000, || {
        decode_type_def(black_box(&encoded), DecodeLimits::default())
            .expect("benchmark wire payload remains valid")
    });

    let (source, target) = migration_snapshots();
    let planner = DefaultMigrationPlanner::default();
    let intent = MigrationIntent::default();
    runner.measure(
        Suite::Modern,
        "Migration diff and plan (1 entity, 2 changes)",
        10_000,
        || {
            planner
                .plan(black_box(&source), black_box(&target), black_box(&intent))
                .expect("benchmark migration remains valid")
        },
    );

    let (request, candidates, limits) = vector_fixture();
    runner.measure(
        Suite::Modern,
        "Exact vector search (128 x 64, top 10)",
        1_000,
        || {
            exact_search(
                black_box(&request),
                black_box(&candidates).iter().cloned(),
                limits,
            )
            .expect("benchmark vector search remains valid")
        },
    );

    let mut memory = MemoryEngine::new();
    let rows = DataSet::try_new((0_u64..128).map(benchmark_account))
        .expect("benchmark data set remains valid");
    memory
        .load(&rows)
        .expect("benchmark data set remains loadable");
    let options = ExecutionOptions::default();
    runner.measure(
        Suite::Modern,
        "Memory pipeline execution (128 input rows)",
        1_000,
        || {
            let mut stream = memory
                .execute(
                    black_box(&pipeline),
                    black_box(&parameters),
                    black_box(&options),
                )
                .expect("benchmark memory execution remains valid");
            collect_stream(&mut stream).expect("benchmark stream remains collectable")
        },
    );

    scale_benchmarks(runner);
}

fn representation_measurements(runner: &mut Runner) {
    runner.note(
        Suite::Historical,
        "Expr handle stack size",
        format!(
            "{} bytes (historical baseline: 32 bytes)",
            size_of::<Expr<Truth>>()
        ),
    );
    runner.note(
        Suite::Historical,
        "Pipeline handle stack size",
        format!(
            "{} bytes (historical baseline: 32 bytes)",
            size_of::<Pipeline<BenchmarkRow>>()
        ),
    );
}

fn historical_expression() -> Expr<Truth> {
    BenchmarkRow::active
        .eq(true)
        .and(BenchmarkRow::value.ge(10_i32))
        .and(BenchmarkRow::label.contains("data"))
}

fn historical_pipeline() -> Pipeline<BenchmarkRow> {
    Pipeline::<BenchmarkRow>::from_model()
        .filter(historical_expression())
        .order_by_desc(BenchmarkRow::id)
        .limit(100)
}

fn historical_row() -> BenchmarkRow {
    BenchmarkRow {
        id: 1,
        value: 42,
        active: true,
        label: "data operating language".into(),
    }
}

fn build_historical_runtime_model() -> ModelDef {
    ModelBuilder::with_key(
        "historical/benchmark_runtime_rows",
        "benchmark_runtime_rows",
    )
    .field::<u64>("id")
    .field::<i32>("value")
    .field::<bool>("active")
    .field::<String>("label")
    .identity(["id"])
    .freeze()
    .expect("historical runtime model remains valid")
}

fn historical_postgres_catalog() -> PostgresCatalog {
    let mapping = TableMapping::new("public", "benchmark_rows")
        .field("id", ColumnMapping::required("id"))
        .field("value", ColumnMapping::required("value"))
        .field("active", ColumnMapping::required("active"))
        .field("label", ColumnMapping::required("label"));
    let mut catalog = PostgresCatalog::new();
    catalog
        .register(
            BenchmarkRow::model_def().expect("historical benchmark model remains valid"),
            mapping,
        )
        .expect("historical PostgreSQL mapping remains valid");
    catalog
}

fn postgres_runtime_engine(catalog: PostgresCatalog) -> PostgresEngine {
    let runtime = PostgresRuntimeConfig::parse(
        "host=localhost user=dol dbname=dol sslmode=require connect_timeout=1",
    )
    .expect("benchmark PostgreSQL runtime policy remains valid");
    PostgresEngine::with_runtime(catalog, runtime)
}

fn scale_benchmarks(runner: &mut Runner) {
    for depth in [8_usize, 64, 256] {
        runner.measure(
            Suite::Modern,
            format!("Expression construction plus preparation (depth {depth})"),
            fixed_scale_iterations(depth),
            || {
                expression_at_depth(depth)
                    .prepare(&dol_core::expr::BindContext::new())
                    .expect("scaled expression remains valid")
            },
        );
    }

    for stages in [4_usize, 32, 256] {
        runner.measure(
            Suite::Modern,
            format!("Pipeline construction plus lowering ({stages} stages)"),
            fixed_scale_iterations(stages),
            || {
                pipeline_with_stages(stages)
                    .logical_plan()
                    .expect("scaled pipeline remains valid")
            },
        );
    }

    for fields in [4_usize, 64, 1_024] {
        runner.measure(
            Suite::Modern,
            format!("Runtime model definition ({fields} fields)"),
            fixed_scale_iterations(fields),
            || model_with_fields(fields),
        );
    }
}

fn fixed_scale_iterations(size: usize) -> u64 {
    match size {
        0..=8 => 10_000,
        9..=64 => 1_000,
        65..=256 => 100,
        _ => 10,
    }
}

fn expression_at_depth(depth: usize) -> Expr<Truth> {
    let mut expression = Expr::literal(Truth::True);
    for _ in 1..depth {
        expression = expression.and(Expr::literal(Truth::True));
    }
    expression
}

fn pipeline_with_stages(stages: usize) -> Pipeline<BenchAccount> {
    let mut pipeline = Pipeline::<BenchAccount>::from_model();
    for limit in 1..stages {
        pipeline = pipeline.limit(u64::try_from(limit).expect("stage count remains in range"));
    }
    pipeline
}

fn model_with_fields(fields: usize) -> ModelDef {
    let mut builder = ModelBuilder::with_key(
        format!("bench/model-width-{fields}"),
        format!("ModelWidth{fields}"),
    );
    for index in 0..fields {
        builder = builder.field::<u64>(format!("field_{index:04}"));
    }
    builder
        .identity(["field_0000"])
        .freeze()
        .expect("scaled runtime model remains valid")
}

fn build_expression() -> Expr<Truth> {
    BenchAccount::active
        .eq(true)
        .and(BenchAccount::balance.ge(10_i64))
        .or(BenchAccount::nickname.is_missing())
}

fn benchmark_pipeline() -> Pipeline<BenchAccount> {
    Pipeline::<BenchAccount>::from_model()
        .filter(build_expression())
        .offset(2)
        .limit(50)
}

fn benchmark_account(id: u64) -> BenchAccount {
    BenchAccount {
        id,
        active: id.is_multiple_of(2),
        balance: i64::try_from(id).expect("benchmark id remains in range") - 20,
        nickname: id.is_multiple_of(3).then(|| format!("account-{id}")),
    }
}

fn build_runtime_model() -> ModelDef {
    ModelBuilder::with_key("bench/account", "BenchAccount")
        .field::<u64>("id")
        .field::<bool>("active")
        .field::<i64>("balance")
        .optional_field::<String>("nickname")
        .identity(["id"])
        .unique(["nickname"])
        .freeze()
        .expect("benchmark runtime model remains valid")
}

fn postgres_catalog() -> PostgresCatalog {
    let mapping = TableMapping::new("public", "accounts")
        .field("id", ColumnMapping::required("id"))
        .field("active", ColumnMapping::required("active"))
        .field("balance", ColumnMapping::required("balance"))
        .field(
            "nickname",
            ColumnMapping::optional("nickname", "nickname_present"),
        );
    let mut catalog = PostgresCatalog::new();
    catalog
        .register(
            BenchAccount::model_def().expect("benchmark model remains valid"),
            mapping,
        )
        .expect("benchmark PostgreSQL mapping remains valid");
    catalog
}

fn mongodb_catalog() -> MongodbCatalog {
    let mapping = CollectionMapping::new("dol_bench", "accounts")
        .field("id", FieldMapping::new("id"))
        .field("active", FieldMapping::new("active"))
        .field("balance", FieldMapping::new("balance"))
        .field("nickname", FieldMapping::new("nickname"));
    let mut catalog = MongodbCatalog::new();
    catalog
        .register(
            BenchAccount::model_def().expect("benchmark model remains valid"),
            mapping,
        )
        .expect("benchmark MongoDB mapping remains valid");
    catalog
}

fn migration_snapshots() -> (CatalogSnapshot, CatalogSnapshot) {
    let scope =
        CatalogScope::try_new("postgres", "public").expect("benchmark catalog scope remains valid");
    let id = CatalogField::try_new("id", "id", "bigint", true, false)
        .expect("benchmark field remains valid");
    let active = CatalogField::try_new("active", "active", "boolean", true, false)
        .expect("benchmark field remains valid");
    let balance = CatalogField::try_new("balance", "balance", "bigint", true, false)
        .expect("benchmark field remains valid");
    let nickname = CatalogField::try_new("nickname", "nickname", "text", false, true)
        .expect("benchmark field remains valid");
    let email = CatalogField::try_new("email", "email", "text", false, true)
        .expect("benchmark field remains valid");
    let by_email = CatalogIndex::try_new("by_email", "accounts_by_email", ["email"], true)
        .expect("benchmark index remains valid");

    let source_entity = CatalogEntity::try_new(
        "account",
        "accounts",
        vec![
            id.clone(),
            active.clone(),
            balance.clone(),
            nickname.clone(),
        ],
        Vec::new(),
    )
    .expect("benchmark source catalog remains valid");
    let target_entity = CatalogEntity::try_new(
        "account",
        "accounts",
        vec![id, active, balance, nickname, email],
        vec![by_email],
    )
    .expect("benchmark target catalog remains valid");

    let source = CatalogSnapshot::try_new(
        scope.clone(),
        CatalogRevision::try_new("bench-source").expect("benchmark source revision remains valid"),
        vec![source_entity],
    )
    .expect("benchmark source snapshot remains valid");
    let target = CatalogSnapshot::try_new(
        scope,
        CatalogRevision::try_new("bench-target").expect("benchmark target revision remains valid"),
        vec![target_entity],
    )
    .expect("benchmark target snapshot remains valid");
    (source, target)
}

fn vector_fixture() -> (ExactSearchRequest, Vec<VectorCandidate>, SearchLimits) {
    const CANDIDATES: usize = 128;
    const DIMENSIONS: usize = 64;

    let limits = SearchLimits::new(CANDIDATES, 10).expect("benchmark limits remain valid");
    let query = Vector::new(
        (0..DIMENSIONS)
            .map(|index| index as f32 / DIMENSIONS as f32)
            .collect(),
    )
    .expect("benchmark query remains valid");
    let request = ExactSearchRequest::new(query, Distance::Euclidean, 10, limits)
        .expect("benchmark search request remains valid");
    let candidates = (0..CANDIDATES)
        .map(|candidate| {
            let vector = Vector::new(
                (0..DIMENSIONS)
                    .map(|dimension| ((candidate * 17 + dimension * 13) % 101) as f32 / 101.0)
                    .collect(),
            )
            .expect("benchmark candidate remains valid");
            VectorCandidate::new(
                VectorId::new(format!("candidate-{candidate:03}"))
                    .expect("benchmark candidate id remains valid"),
                vector,
            )
        })
        .collect();
    (request, candidates, limits)
}
