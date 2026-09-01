use std::sync::{Arc, Barrier};

use dol::core::limits::PipelineLimits;
use dol::core::plan::{LogicalNode, SortDirection};
use dol::prelude::*;

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
        .and(BenchmarkRow::value.ge(10_i32))
        .and(BenchmarkRow::label.contains("data"))
}

fn historical_pipeline() -> Pipeline<BenchmarkRow> {
    Pipeline::<BenchmarkRow>::from_model()
        .filter(historical_expression())
        .order_by_desc(BenchmarkRow::id)
        .limit(100)
}

#[test]
fn historical_fixture_shape_and_truth_result_are_exact() {
    let model = BenchmarkRow::model_def().unwrap();
    assert_eq!(model.name(), "benchmark_rows");
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
        id: 1,
        value: 42,
        active: true,
        label: "data operating language".into(),
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
            limit: Some(100),
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
