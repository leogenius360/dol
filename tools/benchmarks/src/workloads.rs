//! Frozen `closure-v1` workload implementations.
//!
//! Every timed operation uses APIs available in both immutable lineage
//! revisions. Candidate-only cache APIs are intentionally reached through the
//! shared public entry points so the byte-identical overlay compiles on both
//! sides and still observes their different implementations.

use std::hint::black_box;

use dol::prelude::*;
use dol_core::expr::{BindContext, EvalContext};
use dol_core::model::{ModelBuilder, ModelDef, Presence};
use dol_core::value::DataValue;
use dol_engine::{ExecutionOptions, PlacementPolicy, analyze_engine_placement, collect_stream};
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
use dol_wire::{DecodeLimits, decode_type_def, decode_typed_datum, encode_typed_datum};

use crate::canonical::CanonicalRunner;
use crate::scenario::{Lifecycle, ScenarioDefinition, scenario_definition};

#[allow(dead_code)]
#[path = "../../../perf/fixtures/closure-v1/fixture.rs"]
mod closure_fixture;

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

/// Registers and runs every canonical and scaling workload in contract order.
pub fn run_closure_v1(runner: &mut CanonicalRunner) -> Result<(), String> {
    validate_semantic_fixtures()?;
    run_expression_workloads(runner)?;
    run_pipeline_workloads(runner)?;
    run_engine_workloads(runner)?;
    run_scaling_workloads(runner)
}

fn run_expression_workloads(runner: &mut CanonicalRunner) -> Result<(), String> {
    if let Some(definition) = selected(runner, "expr.construct.basic", Lifecycle::Cold) {
        runner.measure(definition, 100_000, 1, build_expression)?;
    }

    if let Some(definition) = selected(runner, "expr.prepare.basic", Lifecycle::Cold) {
        let model = BenchAccount::model_def().map_err(|error| error.to_string())?;
        let expression = build_expression();
        runner.measure(definition, 100_000, 1, || {
            black_box(&expression)
                .prepare_for(black_box(model))
                .expect("closure-v1 expression preparation remains valid")
        })?;
    }

    if let Some(definition) = selected(runner, "expr.eval.prepared", Lifecycle::Prepared) {
        let model = BenchAccount::model_def().map_err(|error| error.to_string())?;
        let expression = build_expression();
        let row = benchmark_account(42);
        let prepared = expression
            .prepare_for(model)
            .map_err(|error| error.to_string())?;
        let context = EvalContext::single(&row);
        runner.measure(definition, 1_000_000, 1, || {
            black_box(&prepared)
                .evaluate_truth(black_box(&context))
                .expect("closure-v1 prepared evaluation remains valid")
        })?;
    }

    if let Some(definition) = selected(runner, "expr.identity", Lifecycle::Cold) {
        runner.measure(definition, 100_000, 1, || {
            build_expression()
                .fingerprint()
                .expect("closure-v1 cold expression identity remains valid")
        })?;
    }

    if let Some(definition) = selected(runner, "expr.identity", Lifecycle::Warm) {
        let expression = build_expression();
        expression
            .fingerprint()
            .map_err(|error| error.to_string())?;
        runner.measure(definition, 100_000, 1, || {
            black_box(&expression)
                .fingerprint()
                .expect("closure-v1 warm expression identity remains valid")
        })?;
    }
    Ok(())
}

fn run_pipeline_workloads(runner: &mut CanonicalRunner) -> Result<(), String> {
    if let Some(definition) = selected(runner, "pipeline.lower", Lifecycle::Cold) {
        runner.measure(definition, 10_000, 1, || {
            benchmark_pipeline()
                .logical_plan()
                .expect("closure-v1 cold pipeline lowering remains valid")
        })?;
    }

    if let Some(definition) = selected(runner, "pipeline.lower", Lifecycle::Warm) {
        let pipeline = benchmark_pipeline();
        pipeline.logical_plan().map_err(|error| error.to_string())?;
        runner.measure(definition, 100_000, 1, || {
            black_box(&pipeline)
                .logical_plan()
                .expect("closure-v1 warm pipeline lowering remains valid")
        })?;
    }

    if let Some(definition) = selected(runner, "pipeline.identity", Lifecycle::Cold) {
        runner.measure(definition, 10_000, 1, || {
            benchmark_pipeline()
                .fingerprint()
                .expect("closure-v1 cold pipeline identity remains valid")
        })?;
    }

    if let Some(definition) = selected(runner, "pipeline.identity", Lifecycle::Warm) {
        let pipeline = benchmark_pipeline();
        pipeline.fingerprint().map_err(|error| error.to_string())?;
        runner.measure(definition, 100_000, 1, || {
            black_box(&pipeline)
                .fingerprint()
                .expect("closure-v1 warm pipeline identity remains valid")
        })?;
    }
    Ok(())
}

fn run_engine_workloads(runner: &mut CanonicalRunner) -> Result<(), String> {
    if let Some(definition) = selected(runner, "postgres.explain", Lifecycle::Fresh) {
        let postgres = postgres_runtime_engine()?;
        runner.measure(definition, 10_000, 1, || {
            postgres
                .explain(&benchmark_pipeline(), PlacementPolicy::RemoteOnly)
                .expect("closure-v1 fresh PostgreSQL explain remains valid")
        })?;
    }

    if let Some(definition) = selected(runner, "postgres.explain", Lifecycle::SharedPlan) {
        let postgres = postgres_runtime_engine()?;
        let pipeline = benchmark_pipeline();
        postgres
            .explain(&pipeline, PlacementPolicy::RemoteOnly)
            .map_err(|error| error.to_string())?;
        runner.measure(definition, 10_000, 1, || {
            postgres
                .explain(black_box(&pipeline), PlacementPolicy::RemoteOnly)
                .expect("closure-v1 shared-plan PostgreSQL explain remains valid")
        })?;
    }

    if let Some(definition) = selected(runner, "postgres.place", Lifecycle::Prepared) {
        let postgres = postgres_runtime_engine()?;
        let plan = benchmark_pipeline()
            .logical_plan()
            .map_err(|error| error.to_string())?;
        runner.measure(definition, 10_000, 1, || {
            analyze_engine_placement(
                black_box(&plan),
                black_box(&postgres),
                PlacementPolicy::RemoteOnly,
            )
            .expect("closure-v1 PostgreSQL placement remains valid")
        })?;
    }

    if let Some(definition) = selected(runner, "postgres.compile.sql", Lifecycle::Prepared) {
        let postgres = PostgresEngine::new(postgres_catalog());
        let plan = benchmark_pipeline()
            .logical_plan()
            .map_err(|error| error.to_string())?;
        let parameters = Parameters::new();
        runner.measure(definition, 10_000, 1, || {
            postgres
                .compile(black_box(&plan), black_box(&parameters))
                .expect("closure-v1 PostgreSQL compilation remains valid")
        })?;
    }

    if let Some(definition) = selected(runner, "wire.decode.v1.84b", Lifecycle::Prepared) {
        let golden = closure_wire_golden()?;
        let decoded =
            decode_type_def(&golden, DecodeLimits::default()).map_err(|error| error.to_string())?;
        if decoded != <Vec<Option<String>> as DataType>::type_def() {
            return Err("pinned closure-v1 wire fixture decoded to the wrong semantic type".into());
        }
        runner.measure(definition, 100_000, golden.len() as u64, || {
            decode_type_def(black_box(&golden), DecodeLimits::default())
                .expect("pinned closure-v1 wire frame remains decodable")
        })?;
    }

    if let Some(definition) = selected(runner, "model.define.runtime", Lifecycle::Cold) {
        runner.measure(definition, 10_000, 4, build_runtime_model)?;
    }

    if let Some(definition) = selected(runner, "mongodb.compile", Lifecycle::Prepared) {
        let mongodb = MongodbEngine::new(mongodb_catalog());
        let plan = benchmark_pipeline()
            .logical_plan()
            .map_err(|error| error.to_string())?;
        let parameters = Parameters::new();
        runner.measure(definition, 10_000, 1, || {
            mongodb
                .compile(black_box(&plan), black_box(&parameters))
                .expect("closure-v1 MongoDB compilation remains valid")
        })?;
    }

    if let Some(definition) = selected(runner, "migration.plan", Lifecycle::Fresh) {
        let (source, target) = migration_snapshots();
        let planner = DefaultMigrationPlanner::default();
        let intent = MigrationIntent::default();
        planner
            .plan(&source, &target, &intent)
            .map_err(|error| error.to_string())?;
        runner.measure(definition, 10_000, 1, || {
            planner
                .plan(black_box(&source), black_box(&target), black_box(&intent))
                .expect("closure-v1 migration planning remains valid")
        })?;
    }

    if let Some(definition) = selected(runner, "vector.search.exact", Lifecycle::Prepared) {
        let (request, candidates, limits) = vector_fixture();
        let validation = exact_search(&request, candidates.iter().cloned(), limits)
            .map_err(|error| error.to_string())?;
        if validation.len() != 10 {
            return Err("closure-v1 vector fixture must return exactly ten matches".into());
        }
        runner.measure(definition, 1_000, candidates.len() as u64, || {
            exact_search(
                black_box(&request),
                black_box(&candidates).iter().cloned(),
                limits,
            )
            .expect("closure-v1 exact vector search remains valid")
        })?;
    }

    run_memory_scenario(runner, "memory.execute", 128)
}

fn run_scaling_workloads(runner: &mut CanonicalRunner) -> Result<(), String> {
    for depth in [8_usize, 64, 256] {
        let definition = scenario(
            match depth {
                8 => "expr.eval.depth.8",
                64 => "expr.eval.depth.64",
                256 => "expr.eval.depth.256",
                _ => unreachable!(),
            },
            Lifecycle::Prepared,
        );
        if !runner.enabled(definition) {
            continue;
        }
        let expression = expression_at_depth(depth);
        let prepared = expression
            .prepare(&BindContext::new())
            .map_err(|error| error.to_string())?;
        let context = EvalContext::new();
        if prepared
            .evaluate_truth(&context)
            .map_err(|error| error.to_string())?
            != Truth::True
        {
            return Err(format!(
                "depth-{depth} expression fixture did not evaluate true"
            ));
        }
        runner.measure(
            definition,
            fixed_scale_iterations(depth),
            depth as u64,
            || {
                prepared
                    .evaluate_truth(black_box(&context))
                    .expect("scaled closure-v1 expression remains evaluable")
            },
        )?;
    }

    for stages in [4_usize, 32, 256] {
        let id = match stages {
            4 => "pipeline.lower.stages.4",
            32 => "pipeline.lower.stages.32",
            256 => "pipeline.lower.stages.256",
            _ => unreachable!(),
        };
        let definition = scenario(id, Lifecycle::Cold);
        if !runner.enabled(definition) {
            continue;
        }
        runner.measure(
            definition,
            fixed_scale_iterations(stages),
            stages as u64,
            || {
                pipeline_with_stages(stages)
                    .logical_plan()
                    .expect("scaled closure-v1 pipeline remains lowerable")
            },
        )?;
    }

    for fields in [4_usize, 64, 1_024] {
        let id = match fields {
            4 => "model.define.width.4",
            64 => "model.define.width.64",
            1_024 => "model.define.width.1024",
            _ => unreachable!(),
        };
        let definition = scenario(id, Lifecycle::Cold);
        if !runner.enabled(definition) {
            continue;
        }
        runner.measure(
            definition,
            fixed_scale_iterations(fields),
            fields as u64,
            || model_with_fields(fields),
        )?;
    }

    run_wire_scaling_scenario(runner, "wire.decode.typed.1k", 1_024)?;
    run_wire_scaling_scenario(runner, "wire.decode.typed.16k", 16_384)?;

    for rows in [1_usize, 16, 256, 4_096, 100_000] {
        let id = match rows {
            1 => "memory.execute.rows.1",
            16 => "memory.execute.rows.16",
            256 => "memory.execute.rows.256",
            4_096 => "memory.execute.rows.4096",
            100_000 => "memory.execute.rows.100000",
            _ => unreachable!(),
        };
        run_memory_scenario(runner, id, rows)?;
    }
    Ok(())
}

fn run_wire_scaling_scenario(
    runner: &mut CanonicalRunner,
    id: &'static str,
    payload_bytes: usize,
) -> Result<(), String> {
    let definition = scenario(id, Lifecycle::Prepared);
    if !runner.enabled(definition) {
        return Ok(());
    }
    let value = "x".repeat(payload_bytes);
    let ty = String::type_def();
    let datum = value.to_datum();
    let encoded =
        encode_typed_datum(&ty, Presence::Required, &datum).map_err(|error| error.to_string())?;
    let decoded =
        decode_typed_datum(&encoded, DecodeLimits::default()).map_err(|error| error.to_string())?;
    if decoded.datum() != &datum {
        return Err(format!(
            "{id} fixture failed its untimed semantic validation"
        ));
    }
    runner.measure(
        definition,
        fixed_scale_iterations(payload_bytes),
        encoded.len() as u64,
        || {
            decode_typed_datum(black_box(&encoded), DecodeLimits::default())
                .expect("scaled closure-v1 typed datum remains decodable")
        },
    )
}

fn run_memory_scenario(
    runner: &mut CanonicalRunner,
    id: &'static str,
    row_count: usize,
) -> Result<(), String> {
    let definition = scenario(id, Lifecycle::Prepared);
    if !runner.enabled(definition) {
        return Ok(());
    }
    let mut memory = MemoryEngine::new();
    let rows = DataSet::try_new((0..row_count).map(|id| {
        benchmark_account(u64::try_from(id).expect("closure-v1 row ID remains in range"))
    }))
    .map_err(|error| error.to_string())?;
    memory.load(&rows).map_err(|error| error.to_string())?;
    let pipeline = benchmark_pipeline();
    let parameters = Parameters::new();
    let options = ExecutionOptions::default();
    let mut validation = memory
        .execute(&pipeline, &parameters, &options)
        .map_err(|error| error.to_string())?;
    let validation = collect_stream(&mut validation).map_err(|error| error.to_string())?;
    let expected = expected_memory_rows(row_count);
    if validation.len() != expected {
        return Err(format!(
            "{id} returned {} rows during semantic validation; expected {expected}",
            validation.len()
        ));
    }
    runner.measure(
        definition,
        fixed_scale_iterations(row_count),
        row_count as u64,
        || {
            let mut stream = memory
                .execute(
                    black_box(&pipeline),
                    black_box(&parameters),
                    black_box(&options),
                )
                .expect("closure-v1 memory execution remains valid");
            collect_stream(&mut stream).expect("closure-v1 memory stream remains collectable")
        },
    )
}

fn expected_memory_rows(row_count: usize) -> usize {
    (0..row_count)
        // `Option::None` materializes as null, not missing, in a DataSet. The
        // fixture's `is_missing` branch therefore remains false for loaded
        // rows; only the active/balance branch selects a row.
        .filter(|id| id.is_multiple_of(2) && *id >= 30)
        .count()
        .saturating_sub(2)
        .min(50)
}

fn validate_semantic_fixtures() -> Result<(), String> {
    let model = BenchAccount::model_def().map_err(|error| error.to_string())?;
    let expression = build_expression();
    let row = benchmark_account(42);
    if expression
        .evaluate_truth(&row)
        .map_err(|error| error.to_string())?
        != Truth::True
    {
        return Err("closure-v1 expression fixture must evaluate true".into());
    }
    let prepared = expression
        .prepare_for(model)
        .map_err(|error| error.to_string())?;
    if prepared
        .evaluate_truth(&EvalContext::single(&row))
        .map_err(|error| error.to_string())?
        != Truth::True
    {
        return Err("closure-v1 prepared expression fixture must evaluate true".into());
    }
    let plan = benchmark_pipeline()
        .logical_plan()
        .map_err(|error| error.to_string())?;
    if plan.nodes().is_empty() {
        return Err("closure-v1 pipeline fixture lowered to an empty plan".into());
    }
    Ok(())
}

fn scenario(id: &'static str, lifecycle: Lifecycle) -> ScenarioDefinition {
    scenario_definition(id, lifecycle)
        .unwrap_or_else(|| panic!("closure-v1 registry is missing `{id}` / `{lifecycle}`"))
}

fn selected(
    runner: &CanonicalRunner,
    id: &'static str,
    lifecycle: Lifecycle,
) -> Option<ScenarioDefinition> {
    let definition = scenario(id, lifecycle);
    runner.enabled(definition).then_some(definition)
}

fn closure_wire_golden() -> Result<[u8; closure_fixture::WIRE_ENCODED_BYTES], String> {
    const GOLDEN: &str =
        include_str!("../../../perf/fixtures/closure-v1/wire/type-def-vec-optional-string-v1.hex");
    let source = GOLDEN.trim().as_bytes();
    let (pairs, remainder) = source.as_chunks::<2>();
    if pairs.len() != closure_fixture::WIRE_ENCODED_BYTES || !remainder.is_empty() {
        return Err("closure-v1 wire golden must contain exactly 84 bytes".into());
    }
    let mut output = [0_u8; closure_fixture::WIRE_ENCODED_BYTES];
    for (slot, pair) in output.iter_mut().zip(pairs) {
        let digits = std::str::from_utf8(pair)
            .map_err(|_| "closure-v1 wire golden must be ASCII".to_string())?;
        *slot = u8::from_str_radix(digits, 16)
            .map_err(|_| "closure-v1 wire golden must contain hexadecimal bytes".to_string())?;
    }
    Ok(output)
}

fn build_expression() -> Expr<Truth> {
    BenchAccount::active
        .eq(true)
        .and(BenchAccount::balance.ge(closure_fixture::MODERN_BALANCE_FLOOR))
        .or(BenchAccount::nickname.is_missing())
}

fn benchmark_pipeline() -> Pipeline<BenchAccount> {
    Pipeline::<BenchAccount>::from_model()
        .filter(build_expression())
        .offset(closure_fixture::MODERN_PIPELINE_OFFSET)
        .limit(closure_fixture::MODERN_PIPELINE_LIMIT)
}

fn benchmark_account(id: u64) -> BenchAccount {
    BenchAccount {
        id,
        active: id.is_multiple_of(2),
        balance: i64::try_from(id).expect("closure-v1 ID remains in range") - 20,
        nickname: id.is_multiple_of(3).then(|| format!("account-{id}")),
    }
}

fn build_runtime_model() -> ModelDef {
    ModelBuilder::with_key(
        closure_fixture::MODERN_MODEL_KEY,
        closure_fixture::MODERN_MODEL_NAME,
    )
    .field::<u64>("id")
    .field::<bool>("active")
    .field::<i64>("balance")
    .optional_field::<String>("nickname")
    .identity(["id"])
    .unique(["nickname"])
    .freeze()
    .expect("closure-v1 runtime model remains valid")
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
        .expect("scaled closure-v1 runtime model remains valid")
}

fn fixed_scale_iterations(size: usize) -> u64 {
    match size {
        0..=8 => 10_000,
        9..=64 => 1_000,
        65..=256 => 100,
        257..=16_384 => 10,
        _ => 1,
    }
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
            BenchAccount::model_def().expect("closure-v1 model remains valid"),
            mapping,
        )
        .expect("closure-v1 PostgreSQL mapping remains valid");
    catalog
}

fn postgres_runtime_engine() -> Result<PostgresEngine, String> {
    let runtime = PostgresRuntimeConfig::parse(
        "host=localhost user=dol dbname=dol sslmode=require connect_timeout=1",
    )
    .map_err(|error| error.to_string())?;
    Ok(PostgresEngine::with_runtime(postgres_catalog(), runtime))
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
            BenchAccount::model_def().expect("closure-v1 model remains valid"),
            mapping,
        )
        .expect("closure-v1 MongoDB mapping remains valid");
    catalog
}

fn migration_snapshots() -> (CatalogSnapshot, CatalogSnapshot) {
    let scope = CatalogScope::try_new("postgres", "public")
        .expect("closure-v1 catalog scope remains valid");
    let id = CatalogField::try_new("id", "id", "bigint", true, false)
        .expect("closure-v1 field remains valid");
    let active = CatalogField::try_new("active", "active", "boolean", true, false)
        .expect("closure-v1 field remains valid");
    let balance = CatalogField::try_new("balance", "balance", "bigint", true, false)
        .expect("closure-v1 field remains valid");
    let nickname = CatalogField::try_new("nickname", "nickname", "text", false, true)
        .expect("closure-v1 field remains valid");
    let email = CatalogField::try_new("email", "email", "text", false, true)
        .expect("closure-v1 field remains valid");
    let by_email = CatalogIndex::try_new("by_email", "accounts_by_email", ["email"], true)
        .expect("closure-v1 index remains valid");
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
    .expect("closure-v1 source catalog remains valid");
    let target_entity = CatalogEntity::try_new(
        "account",
        "accounts",
        vec![id, active, balance, nickname, email],
        vec![by_email],
    )
    .expect("closure-v1 target catalog remains valid");
    let source = CatalogSnapshot::try_new(
        scope.clone(),
        CatalogRevision::try_new("closure-source").expect("revision remains valid"),
        vec![source_entity],
    )
    .expect("closure-v1 source snapshot remains valid");
    let target = CatalogSnapshot::try_new(
        scope,
        CatalogRevision::try_new("closure-target").expect("revision remains valid"),
        vec![target_entity],
    )
    .expect("closure-v1 target snapshot remains valid");
    (source, target)
}

fn vector_fixture() -> (ExactSearchRequest, Vec<VectorCandidate>, SearchLimits) {
    const CANDIDATES: usize = 128;
    const DIMENSIONS: usize = 64;
    let limits = SearchLimits::new(CANDIDATES, 10).expect("closure-v1 limits remain valid");
    let query = Vector::new(
        (0..DIMENSIONS)
            .map(|index| index as f32 / DIMENSIONS as f32)
            .collect(),
    )
    .expect("closure-v1 query remains valid");
    let request = ExactSearchRequest::new(query, Distance::Euclidean, 10, limits)
        .expect("closure-v1 request remains valid");
    let candidates = (0..CANDIDATES)
        .map(|candidate| {
            let vector = Vector::new(
                (0..DIMENSIONS)
                    .map(|dimension| ((candidate * 17 + dimension * 13) % 101) as f32 / 101.0)
                    .collect(),
            )
            .expect("closure-v1 candidate remains valid");
            VectorCandidate::new(
                VectorId::new(format!("candidate-{candidate:03}"))
                    .expect("closure-v1 candidate ID remains valid"),
                vector,
            )
        })
        .collect();
    (request, candidates, limits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_fixture_expectations_cover_slice_and_filter_edges() {
        assert_eq!(expected_memory_rows(1), 0);
        assert_eq!(expected_memory_rows(16), 0);
        assert_eq!(expected_memory_rows(128), 47);
        assert_eq!(expected_memory_rows(256), 50);
    }

    #[test]
    fn pinned_wire_fixture_is_exact_and_semantic() {
        let golden = closure_wire_golden().unwrap();
        assert_eq!(golden.len(), closure_fixture::WIRE_ENCODED_BYTES);
        assert_eq!(closure_fixture::WIRE_SEMANTIC_TYPE, "Vec<Option<String>>");
        assert_eq!(
            decode_type_def(&golden, DecodeLimits::default()).unwrap(),
            <Vec<Option<String>> as DataType>::type_def()
        );
    }

    #[test]
    fn modern_model_fixture_constants_match_the_derived_model() {
        let model = BenchAccount::model_def().unwrap();
        assert_eq!(model.key().as_str(), closure_fixture::MODERN_MODEL_KEY);
        assert_eq!(model.name(), closure_fixture::MODERN_MODEL_NAME);
    }
}
