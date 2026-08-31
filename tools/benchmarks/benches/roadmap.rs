//! Adaptive, dependency-free benchmarks covering the baseline and completed roadmap.

use std::hint::black_box;
use std::mem::size_of;
use std::time::{Duration, Instant};

use dol::prelude::*;
use dol_core::expr::EvalContext;
use dol_core::model::{ModelBuilder, ModelDef};
use dol_engine::{ExecutionOptions, PlacementPolicy, collect_stream};
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

const SAMPLE_COUNT: usize = 9;
const CALIBRATION_FLOOR: Duration = Duration::from_millis(20);
const TARGET_SAMPLE: Duration = Duration::from_millis(150);
const MAX_ITERATIONS: u64 = 50_000_000;

const BASELINE_EXPRESSION_CONSTRUCTION: f64 = 310_598.0;
const BASELINE_EXPRESSION_EVALUATION: f64 = 7_950_175.0;
const BASELINE_CLONE_AND_FINGERPRINTS: f64 = 49_481.0;
const BASELINE_POSTGRES_PLANNING: f64 = 68_660.0;
const BASELINE_RUNTIME_MODEL: f64 = 97_468.0;

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

fn main() {
    println!("DOL roadmap benchmark suite");
    println!(
        "adaptive median: {SAMPLE_COUNT} samples, target {:.0} ms/sample",
        TARGET_SAMPLE.as_secs_f64() * 1_000.0
    );
    println!("baseline: Windows, 2026-08-23, Rust 1.98.0, Cargo bench profile");

    baseline_benchmarks();
    roadmap_benchmarks();
    representation_measurements();
}

fn baseline_benchmarks() {
    println!("\nReconstructed baseline-label workloads");
    println!("reference deltas are directional: the original fixture source is unavailable");

    measure(
        "Expression construction",
        Some(BASELINE_EXPRESSION_CONSTRUCTION),
        build_expression,
    );

    let model = BenchAccount::model_def().expect("benchmark model remains valid");
    let prepared = build_expression()
        .prepare_for(model)
        .expect("benchmark expression remains valid");
    let row = benchmark_account(42);
    let context = EvalContext::single(&row);
    measure(
        "Expression evaluation",
        Some(BASELINE_EXPRESSION_EVALUATION),
        || {
            prepared
                .evaluate_truth(black_box(&context))
                .expect("benchmark evaluation remains valid")
        },
    );

    let expression = build_expression();
    let pipeline = benchmark_pipeline();
    measure(
        "Expression/pipeline clone plus fingerprints",
        Some(BASELINE_CLONE_AND_FINGERPRINTS),
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

    let postgres = postgres_runtime_engine();
    measure(
        "PostgreSQL capability analysis and planning",
        Some(BASELINE_POSTGRES_PLANNING),
        || {
            postgres
                .explain(black_box(&pipeline), PlacementPolicy::RemoteOnly)
                .expect("benchmark plan remains supported")
        },
    );

    measure(
        "Runtime model definition",
        Some(BASELINE_RUNTIME_MODEL),
        build_runtime_model,
    );
}

fn roadmap_benchmarks() {
    println!("\nAdditional roadmap workloads");

    let pipeline = benchmark_pipeline();
    let plan = pipeline
        .logical_plan()
        .expect("benchmark pipeline remains valid");
    let parameters = Parameters::new();
    let postgres = PostgresEngine::new(postgres_catalog());
    measure("PostgreSQL offline compilation", None, || {
        postgres
            .compile(black_box(&plan), black_box(&parameters))
            .expect("benchmark PostgreSQL compilation remains valid")
    });

    let mongodb = MongodbEngine::new(mongodb_catalog());
    measure("MongoDB offline compilation", None, || {
        mongodb
            .compile(black_box(&plan), black_box(&parameters))
            .expect("benchmark MongoDB compilation remains valid")
    });

    let wire_type = <Vec<Option<String>> as DataType>::type_def();
    let encoded = encode_type_def(&wire_type).expect("benchmark wire type remains valid");
    let wire_name = format!("Wire TypeDef decode ({} byte frame)", encoded.len());
    measure(&wire_name, None, || {
        decode_type_def(black_box(&encoded), DecodeLimits::default())
            .expect("benchmark wire payload remains valid")
    });

    let (source, target) = migration_snapshots();
    let planner = DefaultMigrationPlanner::default();
    let intent = MigrationIntent::default();
    measure(
        "Migration diff and plan (1 entity, 2 changes)",
        None,
        || {
            planner
                .plan(black_box(&source), black_box(&target), black_box(&intent))
                .expect("benchmark migration remains valid")
        },
    );

    let (request, candidates, limits) = vector_fixture();
    measure("Exact vector search (128 x 64, top 10)", None, || {
        exact_search(
            black_box(&request),
            black_box(&candidates).iter().cloned(),
            limits,
        )
        .expect("benchmark vector search remains valid")
    });

    let mut memory = MemoryEngine::new();
    let rows = DataSet::try_new((0_u64..128).map(benchmark_account))
        .expect("benchmark data set remains valid");
    memory
        .load(&rows)
        .expect("benchmark data set remains loadable");
    let options = ExecutionOptions::default();
    measure("Memory pipeline execution (128 input rows)", None, || {
        let mut stream = memory
            .execute(
                black_box(&pipeline),
                black_box(&parameters),
                black_box(&options),
            )
            .expect("benchmark memory execution remains valid");
        collect_stream(&mut stream).expect("benchmark stream remains collectable")
    });
}

fn representation_measurements() {
    println!("\nRepresentation");
    report_size::<Expr<Truth>>("Expr handle stack size", 32);
    report_size::<Pipeline<BenchAccount>>("Pipeline handle stack size", 32);
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

fn postgres_runtime_engine() -> PostgresEngine {
    let runtime = PostgresRuntimeConfig::parse(
        "host=localhost user=dol dbname=dol sslmode=require connect_timeout=1",
    )
    .expect("benchmark PostgreSQL runtime policy remains valid");
    PostgresEngine::with_runtime(postgres_catalog(), runtime)
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

fn measure<T>(name: &str, baseline: Option<f64>, mut operation: impl FnMut() -> T) {
    let iterations = calibrate(&mut operation);
    let mut samples = Vec::with_capacity(SAMPLE_COUNT);
    for _ in 0..SAMPLE_COUNT {
        let started = Instant::now();
        for _ in 0..iterations {
            black_box(operation());
        }
        samples.push(started.elapsed().as_secs_f64());
    }

    samples.sort_by(f64::total_cmp);
    let median_seconds = samples[SAMPLE_COUNT / 2];
    let operations_per_second = iterations as f64 / median_seconds;
    let nanos_per_operation = median_seconds * 1_000_000_000.0 / iterations as f64;
    let minimum = iterations as f64 / samples[SAMPLE_COUNT - 1];
    let maximum = iterations as f64 / samples[0];

    match baseline {
        Some(baseline) => {
            let delta = (operations_per_second / baseline - 1.0) * 100.0;
            println!(
                "{name}: {operations_per_second:.0} ops/s ({nanos_per_operation:.1} ns/op, {delta:+.1}% vs baseline; range {minimum:.0}..{maximum:.0}; n={iterations})"
            );
        }
        None => println!(
            "{name}: {operations_per_second:.0} ops/s ({nanos_per_operation:.1} ns/op; range {minimum:.0}..{maximum:.0}; n={iterations})"
        ),
    }
}

fn calibrate<T>(operation: &mut impl FnMut() -> T) -> u64 {
    let mut iterations = 1_u64;
    loop {
        let started = Instant::now();
        for _ in 0..iterations {
            black_box(operation());
        }
        let elapsed = started.elapsed();
        if elapsed >= CALIBRATION_FLOOR || iterations == MAX_ITERATIONS {
            let elapsed_nanos = elapsed.as_nanos().max(1);
            let target = TARGET_SAMPLE.as_nanos();
            let scaled = u128::from(iterations)
                .saturating_mul(target)
                .div_ceil(elapsed_nanos);
            return u64::try_from(scaled)
                .unwrap_or(MAX_ITERATIONS)
                .clamp(1, MAX_ITERATIONS);
        }
        iterations = iterations.saturating_mul(10).min(MAX_ITERATIONS);
    }
}

fn report_size<T>(name: &str, baseline_bytes: usize) {
    let current = size_of::<T>();
    let delta = isize::try_from(current).expect("stack size remains in range")
        - isize::try_from(baseline_bytes).expect("baseline size remains in range");
    println!("{name}: {current} bytes ({delta:+} bytes vs baseline)");
}
