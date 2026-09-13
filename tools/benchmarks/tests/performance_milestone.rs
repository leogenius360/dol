use std::sync::{Arc, Barrier};

use dol::core::limits::PipelineLimits;
use dol::core::plan::{LogicalNode, SortDirection};
use dol::prelude::*;

#[allow(dead_code)]
#[path = "../../../perf/fixtures/closure-v1/fixture.rs"]
mod closure_fixture;

#[derive(Clone, Debug, PartialEq, dol::Model)]
#[dol(key = "historical/benchmark_rows", name = "benchmark_rows")]
struct BenchmarkRow {
    #[dol(identity)]
    id: u64,
    value: i32,
    active: bool,
    label: String,
}

fn historical_expression() -> Expr<Truth> {
    BenchmarkRow::active
        .eq(true)
        .and(BenchmarkRow::value.ge(closure_fixture::HISTORICAL_VALUE_FLOOR))
        .and(BenchmarkRow::label.contains(closure_fixture::HISTORICAL_TEXT_NEEDLE))
}

fn historical_pipeline() -> Pipeline<BenchmarkRow> {
    Pipeline::<BenchmarkRow>::from_model()
        .filter(historical_expression())
        .order_by_desc(BenchmarkRow::id)
        .limit(closure_fixture::HISTORICAL_PIPELINE_LIMIT)
}

#[test]
fn historical_fixture_shape_and_truth_result_are_exact() {
    let model = BenchmarkRow::model_def().unwrap();
    assert_eq!(model.key().as_str(), closure_fixture::HISTORICAL_MODEL_KEY);
    assert_eq!(model.name(), closure_fixture::HISTORICAL_MODEL_NAME);
    assert_eq!(
        model
            .fields()
            .iter()
            .map(|field| field.key().as_str())
            .collect::<Vec<_>>(),
        ["active", "id", "label", "value"]
    );
    assert_eq!(model.identity().unwrap().fields()[0].as_str(), "id");

    let row = BenchmarkRow {
        id: closure_fixture::HISTORICAL_ID,
        value: closure_fixture::HISTORICAL_VALUE,
        active: closure_fixture::HISTORICAL_ACTIVE,
        label: closure_fixture::HISTORICAL_LABEL.into(),
    };
    let expression = historical_expression();
    let prepared = expression.prepare_for(model).unwrap();
    assert_eq!(expression.evaluate_truth(&row).unwrap(), Truth::True);
    assert_eq!(
        prepared
            .evaluate_truth(&dol::core::expr::EvalContext::single(&row))
            .unwrap(),
        Truth::True
    );

    let plan = historical_pipeline().logical_plan().unwrap();
    assert_eq!(plan.nodes().len(), 4);
    assert!(matches!(plan.nodes()[0], LogicalNode::Source { .. }));
    assert!(matches!(plan.nodes()[1], LogicalNode::Filter { .. }));
    assert!(matches!(
        plan.nodes()[2],
        LogicalNode::Sort { ref keys, .. }
            if keys.len() == 1 && keys[0].direction() == SortDirection::Descending
    ));
    assert!(matches!(
        plan.nodes()[3],
        LogicalNode::Slice {
            offset: 0,
            limit: Some(closure_fixture::HISTORICAL_PIPELINE_LIMIT),
            ..
        }
    ));
}

#[test]
fn successful_expression_fingerprint_is_stable_under_concurrent_first_access() {
    let expression = historical_expression();
    let expected = historical_expression().fingerprint().unwrap();
    let barrier = Arc::new(Barrier::new(8));
    let threads = (0..8)
        .map(|_| {
            let expression = expression.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                expression.fingerprint().unwrap()
            })
        })
        .collect::<Vec<_>>();

    for thread in threads {
        assert_eq!(thread.join().unwrap(), expected);
    }
    assert_eq!(expression.fingerprint().unwrap(), expected);
}

#[test]
fn default_pipeline_plan_is_shared_and_custom_limits_remain_uncached() {
    let pipeline = historical_pipeline();
    let first = pipeline.prepared_plan().unwrap();
    let second = pipeline.prepared_plan().unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(
        first.fingerprint(),
        pipeline.logical_plan().unwrap().fingerprint()
    );
    assert_eq!(
        first.fingerprint(),
        historical_pipeline().fingerprint().unwrap()
    );

    let restrictive = PipelineLimits {
        max_nodes: 3,
        ..PipelineLimits::default()
    };
    let first_error = pipeline.logical_plan_with_limits(restrictive).unwrap_err();
    let second_error = pipeline.logical_plan_with_limits(restrictive).unwrap_err();
    assert_eq!(first_error.code(), "PIPELINE-LIMIT-002");
    assert_eq!(second_error.code(), first_error.code());
}

#[test]
fn pipeline_plan_cache_supports_concurrent_first_access() {
    let pipeline = historical_pipeline();
    let barrier = Arc::new(Barrier::new(8));
    let threads = (0..8)
        .map(|_| {
            let pipeline = pipeline.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                pipeline.prepared_plan().unwrap()
            })
        })
        .collect::<Vec<_>>();
    let plans = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect::<Vec<_>>();

    assert!(plans.iter().all(|plan| Arc::ptr_eq(plan, &plans[0])));
    assert!(Arc::ptr_eq(&pipeline.prepared_plan().unwrap(), &plans[0]));
}

fn decode_hex<const N: usize>(fixture: &str) -> [u8; N] {
    let encoded = fixture.trim();
    assert_eq!(encoded.len(), N * 2, "fixture must contain {N} bytes");
    let mut decoded = [0_u8; N];
    let (chunks, remainder) = encoded.as_bytes().as_chunks::<2>();
    assert!(remainder.is_empty(), "hex fixture length must be even");
    for (index, chunk) in chunks.iter().enumerate() {
        let digits = std::str::from_utf8(chunk).expect("hex fixture must be ASCII");
        decoded[index] = u8::from_str_radix(digits, 16).expect("fixture must contain only hex");
    }
    decoded
}

#[test]
fn closure_v1_expression_fingerprint_is_pinned_independently() {
    const EXPECTED_HEX: &str =
        include_str!("../../../perf/fixtures/closure-v1/fingerprint/historical-expression-v1.hex");
    let expected = decode_hex::<32>(EXPECTED_HEX);

    assert_eq!(
        historical_expression().fingerprint().unwrap().as_bytes(),
        &expected,
        "closure-v1 fingerprint changes require a new contract identifier"
    );
}

#[cfg(target_arch = "x86_64")]
#[test]
fn public_performance_handle_layout_is_pinned_on_x86_64() {
    assert_eq!(
        std::mem::size_of::<Expr<Truth>>(),
        24,
        "the public Expr layout is part of closure-v1"
    );
    assert_eq!(
        std::mem::size_of::<Pipeline<BenchmarkRow>>(),
        8,
        "the public Pipeline layout is part of closure-v1"
    );
}
