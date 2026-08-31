use dol::core::plan::{LogicalNode, PlanOutput, RowsFrame, WindowFrameBound, WindowFunctionKind};
use dol::core::types::DataType;
use dol::prelude::*;

#[derive(dol::Model)]
#[dol(key = "example/slice4-user", name = "Slice4User")]
struct User {
    #[dol(identity)]
    id: u64,
    organization_id: u64,
    score: i64,
    manager_id: u64,
    tags: Vec<String>,
    optional_tags: Option<Vec<String>>,
    quota: Option<i64>,
}

#[derive(dol::Model)]
#[dol(key = "example/slice4-order", name = "Slice4Order")]
struct Order {
    #[dol(identity)]
    id: u64,
    user_id: u64,
    amount: i64,
}

#[test]
fn existential_subqueries_infer_correlation_from_normal_filters() {
    let existence: Expr<Truth> = Pipeline::<Order>::from_model()
        .filter(Order::amount.gt(0))
        .exists();
    let uncorrelated = Pipeline::<User>::from_model().filter(existence);
    let uncorrelated_plan = uncorrelated.logical_plan().unwrap();
    let LogicalNode::Filter { condition, .. } = uncorrelated_plan.nodes().last().unwrap() else {
        panic!("expected existential truth filter");
    };
    assert_eq!(condition.existential_dependencies().len(), 1);
    assert!(matches!(uncorrelated_plan.output(), PlanOutput::Model(_)));

    let correlated = Pipeline::<User>::from_model().filter(
        Pipeline::<Order>::from_model()
            .filter(Order::user_id.eq(User::id).and(Order::amount.gt(100)))
            .exists(),
    );
    let correlated_plan = correlated.logical_plan().unwrap();
    let LogicalNode::Filter { condition, .. } = correlated_plan.nodes().last().unwrap() else {
        panic!("expected correlated existential truth filter");
    };
    let dependencies = condition.existential_dependencies();
    assert_eq!(dependencies.len(), 1);
    let LogicalNode::Filter {
        condition: nested_condition,
        ..
    } = &correlated_plan.nodes()[dependencies[0].index()]
    else {
        panic!("expected correlated nested filter");
    };
    assert_eq!(nested_condition.outer_scope_count(), 1);
    assert_eq!(nested_condition.scope_count(), 2);

    let not_exists = Pipeline::<User>::from_model().filter(
        Pipeline::<Order>::from_model()
            .filter(Order::user_id.eq(User::id))
            .not_exists(),
    );
    let not_exists_plan = not_exists.logical_plan().unwrap();
    assert!(matches!(
        not_exists_plan.nodes().last().unwrap(),
        LogicalNode::Filter { .. }
    ));
    assert_ne!(correlated_plan.fingerprint(), not_exists_plan.fingerprint());
}

#[test]
fn chained_and_combined_filters_have_the_same_correlated_identity() {
    let chained = Pipeline::<User>::from_model().filter(
        Pipeline::<Order>::from_model()
            .filter(Order::user_id.eq(User::id))
            .filter(Order::amount.gt(100))
            .exists(),
    );
    let combined = Pipeline::<User>::from_model().filter(
        Pipeline::<Order>::from_model()
            .filter(Order::user_id.eq(User::id).and(Order::amount.gt(100)))
            .exists(),
    );

    assert_eq!(
        chained.fingerprint().unwrap(),
        combined.fingerprint().unwrap()
    );
}

#[test]
fn adjacent_filters_canonicalize_without_creating_left_deep_expression_trees() {
    let mut pipeline = Pipeline::<User>::from_model();
    for threshold in 0_i64..512 {
        pipeline = pipeline.filter(User::score.gt(threshold));
    }

    assert_eq!(pipeline.stage_count(), 2);
    pipeline.logical_plan().unwrap();
}

#[test]
fn correlated_filter_keeps_its_exact_pipeline_position() {
    let plan = Pipeline::<User>::from_model()
        .filter(
            Pipeline::<Order>::from_model()
                .filter(Order::user_id.eq(User::id))
                .limit(10)
                .exists(),
        )
        .logical_plan()
        .unwrap();

    let LogicalNode::Filter { condition, .. } = plan.nodes().last().unwrap() else {
        panic!("expected outer existential filter");
    };
    let dependency = condition.existential_dependencies()[0];
    let LogicalNode::Slice { input, .. } = &plan.nodes()[dependency.index()] else {
        panic!("expected nested slice root");
    };
    let LogicalNode::Filter {
        condition: nested_condition,
        ..
    } = &plan.nodes()[input.index()]
    else {
        panic!("expected correlated filter below the slice");
    };
    assert_eq!(nested_condition.outer_scope_count(), 1);
}

#[test]
fn correlated_existential_authoring_fingerprint_does_not_require_outer_binding() {
    let build = || {
        Pipeline::<Order>::from_model()
            .filter(Order::user_id.eq(User::id).and(Order::amount.gt(100)))
            .exists()
    };

    assert_eq!(
        build().fingerprint().unwrap(),
        build().fingerprint().unwrap()
    );
}

#[test]
fn non_filter_nested_stages_do_not_capture_outer_scope() {
    let pipeline = Pipeline::<User>::from_model()
        .filter(Pipeline::<Order>::from_model().order_by(User::id).exists());
    assert_eq!(
        pipeline.logical_plan().unwrap_err().code(),
        "EXPR-SCOPE-001"
    );
}

#[test]
fn existential_truth_composes_with_normal_boolean_expressions() {
    let has_order = Pipeline::<Order>::from_model()
        .filter(Order::user_id.eq(User::id))
        .exists();
    let has_large_order = Pipeline::<Order>::from_model()
        .filter(Order::user_id.eq(User::id).and(Order::amount.gt(100)))
        .exists();

    let plan = Pipeline::<User>::from_model()
        .filter(User::score.gt(0).and(has_order.or(has_large_order)))
        .logical_plan()
        .unwrap();
    let LogicalNode::Filter { condition, .. } = plan.nodes().last().unwrap() else {
        panic!("expected composable existential filter");
    };
    assert_eq!(condition.existential_dependencies().len(), 2);
}

#[test]
fn outer_references_require_an_existential_boundary() {
    let correlated = Pipeline::<Order>::from_model().filter(Order::user_id.eq(User::id));
    assert_eq!(
        correlated.logical_plan().unwrap_err().code(),
        "EXPR-SCOPE-001"
    );
}

#[test]
fn correlated_self_subqueries_use_aliases_but_alias_spelling_is_not_identity() {
    let build = |outer: &'static str, inner: &'static str| {
        Pipeline::<User>::from_model_as(outer).filter(
            Pipeline::<User>::from_model_as(inner)
                .filter(User::manager_id.at(outer).eq(User::id.at(inner)))
                .exists(),
        )
    };

    assert_eq!(
        build("employee", "manager").fingerprint().unwrap(),
        build("e", "m").fingerprint().unwrap()
    );

    let error = Pipeline::<User>::from_model().filter(
        Pipeline::<User>::from_model()
            .filter(User::manager_id.eq(User::id))
            .exists(),
    );
    assert_eq!(
        error.logical_plan().unwrap_err().code(),
        "PIPELINE-SCOPE-001"
    );
}

#[test]
fn unnest_expands_list_outputs_and_preserves_element_semantics() {
    let tags: Pipeline<String> = Pipeline::<User>::from_model().select(User::tags).unnest();
    let plan = tags.logical_plan().unwrap();
    let LogicalNode::Unnest { element, .. } = plan.nodes().last().unwrap() else {
        panic!("expected unnest node");
    };
    assert_eq!(element, &String::type_def());
    assert!(matches!(plan.output(), PlanOutput::Value(ty) if ty == &String::type_def()));

    let optional: Pipeline<String> = Pipeline::<User>::from_model()
        .select(User::optional_tags)
        .unnest_present();
    optional.logical_plan().unwrap();
}

#[test]
fn ranking_windows_lock_partition_and_peer_order_semantics() {
    let ranked: Pipeline<(User, u64)> = Pipeline::<User>::from_model().window(
        rank()
            .partition_by(User::organization_id)
            .order_by_desc(User::score),
    );
    let plan = ranked.logical_plan().unwrap();
    let LogicalNode::Window { window, .. } = plan.nodes().last().unwrap() else {
        panic!("expected window node");
    };
    assert_eq!(window.kind(), WindowFunctionKind::Rank);
    assert!(window.partition().is_some());
    assert_eq!(window.order().len(), 1);
    assert!(window.frame().is_none());
    assert!(matches!(
        plan.output(),
        PlanOutput::Product(outputs)
            if matches!(&outputs[0], PlanOutput::Model(_))
                && matches!(&outputs[1], PlanOutput::Value(ty) if ty == &u64::type_def())
    ));

    let missing_total_order = Pipeline::<User>::from_model()
        .window(row_number().order_by(User::score))
        .logical_plan()
        .unwrap_err();
    assert_eq!(missing_total_order.code(), "PIPELINE-WINDOW-017");

    Pipeline::<User>::from_model()
        .window(row_number().order_by(User::score).order_by(User::id))
        .logical_plan()
        .unwrap();
}

#[test]
fn aggregate_windows_require_explicit_frames_and_exact_null_semantics() {
    let full_partition: Pipeline<(User, Option<i64>)> = Pipeline::<User>::from_model().window(
        window_sum(User::score)
            .partition_by(User::organization_id)
            .rows(RowsFrame::all()),
    );
    full_partition.logical_plan().unwrap();

    let running: Pipeline<(User, Option<i64>)> = Pipeline::<User>::from_model().window(
        window_sum(User::score)
            .order_by(User::id)
            .rows(RowsFrame::to_current()),
    );
    running.logical_plan().unwrap();

    let missing_frame = Pipeline::<User>::from_model()
        .window(window_sum(User::score).order_by(User::id))
        .logical_plan()
        .unwrap_err();
    assert_eq!(missing_frame.code(), "PIPELINE-WINDOW-011");

    let implicit_nullable = Pipeline::<User>::from_model()
        .window(window_sum(User::quota).rows(RowsFrame::all()))
        .logical_plan()
        .unwrap_err();
    assert_eq!(implicit_nullable.code(), "PIPELINE-WINDOW-008");

    Pipeline::<User>::from_model()
        .window(window_sum_present(User::quota).rows(RowsFrame::all()))
        .logical_plan()
        .unwrap();

    let reversed = RowsFrame::new(
        WindowFrameBound::Following(1),
        WindowFrameBound::Preceding(1),
    );
    let invalid_frame = Pipeline::<User>::from_model()
        .window(window_sum(User::score).order_by(User::id).rows(reversed))
        .logical_plan()
        .unwrap_err();
    assert_eq!(invalid_frame.code(), "PIPELINE-WINDOW-013");
}

#[test]
fn conservative_optimizer_removes_only_proven_redundancy() {
    let plan = Pipeline::<User>::from_model()
        .distinct()
        .distinct()
        .offset(0)
        .logical_plan()
        .unwrap();
    let optimized = plan.optimized().unwrap();

    assert_eq!(plan.fingerprint(), optimized.fingerprint());
    assert_eq!(plan.output(), optimized.output());
    assert_eq!(optimized.nodes().len(), 2);
    assert!(matches!(
        optimized.nodes().last(),
        Some(LogicalNode::Distinct { .. })
    ));

    let optimized_again = optimized.optimized().unwrap();
    assert_eq!(optimized.nodes().len(), optimized_again.nodes().len());
    assert_eq!(optimized.fingerprint(), optimized_again.fingerprint());
}

#[test]
fn window_function_and_frame_semantics_participate_in_identity() {
    let ranked = Pipeline::<User>::from_model()
        .window(rank().order_by(User::score))
        .fingerprint()
        .unwrap();
    let dense = Pipeline::<User>::from_model()
        .window(dense_rank().order_by(User::score))
        .fingerprint()
        .unwrap();
    assert_ne!(ranked, dense);

    let full = Pipeline::<User>::from_model()
        .window(window_sum(User::score).rows(RowsFrame::all()))
        .fingerprint()
        .unwrap();
    let running = Pipeline::<User>::from_model()
        .window(
            window_sum(User::score)
                .order_by(User::id)
                .rows(RowsFrame::to_current()),
        )
        .fingerprint()
        .unwrap();
    assert_ne!(full, running);
}

#[test]
fn optimizer_remaps_existential_dependencies_after_rewrites() {
    let subquery = Pipeline::<Order>::from_model()
        .distinct()
        .distinct()
        .offset(0);
    let plan = Pipeline::<User>::from_model()
        .filter(subquery.filter(User::id.eq(Order::user_id)).exists())
        .offset(0)
        .logical_plan()
        .unwrap();
    let optimized = plan.optimized().unwrap();

    assert_eq!(plan.fingerprint(), optimized.fingerprint());
    assert!(optimized.nodes().len() < plan.nodes().len());
    let LogicalNode::Filter { input, condition } = optimized.nodes().last().unwrap() else {
        panic!("expected optimized existential filter");
    };
    let dependencies = condition.existential_dependencies();
    assert_eq!(dependencies.len(), 1);
    let subquery = dependencies[0];
    assert!(input.index() < optimized.nodes().len());
    assert!(subquery.index() < optimized.nodes().len());
    let Some(LogicalNode::Filter {
        input: nested_input,
        condition: nested_condition,
    }) = optimized.nodes().get(subquery.index())
    else {
        panic!("expected correlated nested filter");
    };
    assert_eq!(nested_condition.outer_scope_count(), 1);
    assert!(matches!(
        optimized.nodes().get(nested_input.index()),
        Some(LogicalNode::Distinct { .. })
    ));
}

#[test]
fn row_frames_canonicalize_zero_distance_and_total_order_rejects_duplicate_identity_risk() {
    let zero_distance = RowsFrame::new(
        WindowFrameBound::Preceding(0),
        WindowFrameBound::Following(0),
    );
    let current = RowsFrame::new(WindowFrameBound::CurrentRow, WindowFrameBound::CurrentRow);
    assert_eq!(zero_distance, current);

    let zero_fingerprint = Pipeline::<User>::from_model()
        .window(
            window_sum(User::score)
                .order_by(User::id)
                .rows(zero_distance),
        )
        .fingerprint()
        .unwrap();
    let current_fingerprint = Pipeline::<User>::from_model()
        .window(window_sum(User::score).order_by(User::id).rows(current))
        .fingerprint()
        .unwrap();
    assert_eq!(zero_fingerprint, current_fingerprint);

    let duplicate_identity_risk = Pipeline::<User>::from_model()
        .union_all(Pipeline::<User>::from_model())
        .window(row_number().order_by(User::id))
        .logical_plan()
        .unwrap_err();
    assert_eq!(duplicate_identity_risk.code(), "PIPELINE-WINDOW-018");

    Pipeline::<User>::from_model()
        .intersect(Pipeline::<User>::from_model())
        .window(row_number().order_by(User::id))
        .logical_plan()
        .unwrap();
}
