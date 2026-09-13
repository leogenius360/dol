//! Criterion-only exploratory diagnostics; never a closure verdict source.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use dol::prelude::*;
use dol_core::expr::EvalContext;
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

fn evaluator_diagnostics(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("evaluator");
    group.bench_function("construct", |bencher| bencher.iter(build_expression));

    let model = BenchAccount::model_def().expect("exploratory model remains valid");
    let expression = build_expression();
    group.bench_function("prepare", |bencher| {
        bencher.iter(|| {
            black_box(&expression)
                .prepare_for(black_box(model))
                .expect("exploratory expression remains preparable")
        });
    });

    let prepared = expression
        .prepare_for(model)
        .expect("exploratory expression remains valid");
    let row = benchmark_account(42);
    let context = EvalContext::single(&row);
    group.bench_function("prepared-evaluate", |bencher| {
        bencher.iter(|| {
            black_box(&prepared)
                .evaluate_truth(black_box(&context))
                .expect("exploratory prepared evaluation remains valid")
        });
    });
    group.finish();
}

fn identity_and_pipeline_diagnostics(criterion: &mut Criterion) {
    let mut identity = criterion.benchmark_group("identity");
    identity.bench_function("expression-cold", |bencher| {
        bencher.iter(|| {
            build_expression()
                .fingerprint()
                .expect("exploratory expression fingerprint remains valid")
        });
    });
    let expression = build_expression();
    expression
        .fingerprint()
        .expect("exploratory warm expression remains valid");
    identity.bench_function("expression-warm", |bencher| {
        bencher.iter(|| {
            black_box(&expression)
                .fingerprint()
                .expect("exploratory expression fingerprint remains valid")
        });
    });
    identity.bench_function("pipeline-cold", |bencher| {
        bencher.iter(|| {
            benchmark_pipeline()
                .fingerprint()
                .expect("exploratory pipeline fingerprint remains valid")
        });
    });
    let pipeline = benchmark_pipeline();
    pipeline
        .fingerprint()
        .expect("exploratory warm pipeline remains valid");
    identity.bench_function("pipeline-warm", |bencher| {
        bencher.iter(|| {
            black_box(&pipeline)
                .fingerprint()
                .expect("exploratory pipeline fingerprint remains valid")
        });
    });
    identity.finish();

    let mut lowering = criterion.benchmark_group("pipeline");
    lowering.bench_function("logical-plan-cold", |bencher| {
        bencher.iter(|| {
            benchmark_pipeline()
                .logical_plan()
                .expect("exploratory pipeline remains lowerable")
        });
    });
    let pipeline = benchmark_pipeline();
    pipeline
        .prepared_plan()
        .expect("exploratory prepared plan remains valid");
    lowering.bench_function("logical-plan-clone-warm", |bencher| {
        bencher.iter(|| {
            black_box(&pipeline)
                .logical_plan()
                .expect("exploratory cached plan remains cloneable")
        });
    });
    lowering.bench_function("prepared-plan-share-warm", |bencher| {
        bencher.iter(|| {
            black_box(&pipeline)
                .prepared_plan()
                .expect("exploratory cached plan remains shareable")
        });
    });
    lowering.finish();
}

fn wire_diagnostics(criterion: &mut Criterion) {
    let semantic_type = <Vec<Option<String>> as DataType>::type_def();
    let encoded = encode_type_def(&semantic_type).expect("exploratory wire type remains encodable");
    assert_eq!(
        encoded.len(),
        84,
        "closure-v1 wire fixture must remain 84 bytes"
    );

    let mut group = criterion.benchmark_group("wire");
    group.bench_function("type-def-encode", |bencher| {
        bencher.iter(|| {
            encode_type_def(black_box(&semantic_type))
                .expect("exploratory wire type remains encodable")
        });
    });
    group.bench_function("type-def-decode-84b", |bencher| {
        bencher.iter(|| {
            decode_type_def(black_box(&encoded), DecodeLimits::default())
                .expect("exploratory wire frame remains decodable")
        });
    });
    group.finish();
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
        balance: i64::try_from(id).expect("exploratory ID remains in range") - 20,
        nickname: id.is_multiple_of(3).then(|| format!("account-{id}")),
    }
}

criterion_group!(
    explore,
    evaluator_diagnostics,
    identity_and_pipeline_diagnostics,
    wire_diagnostics
);
criterion_main!(explore);
