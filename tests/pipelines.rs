use dol::core::plan::{LogicalNode, PlanOutput, ProjectionKind, SortDirection};
use dol::core::types::{DataType, ScalarRepr, TypeDef, TypeProperties};
use dol::core::value::{DataValue, DatumRef, ValueRef};
use dol::prelude::*;

#[derive(Clone, Debug)]
struct OpaqueCode(String);

impl DataType for OpaqueCode {
    fn type_def() -> TypeDef {
        let mut properties = TypeProperties::none();
        properties.equality = true;
        properties.keyable = true;
        TypeDef::scalar("example/opaque-code", 1, ScalarRepr::String).with_properties(properties)
    }
}

impl DataValue for OpaqueCode {
    fn datum_ref(&self) -> DatumRef<'_> {
        DatumRef::Value(ValueRef::String(&self.0))
    }
}

#[derive(Clone, Debug)]
struct NoEqualityCode(String);

impl DataType for NoEqualityCode {
    fn type_def() -> TypeDef {
        TypeDef::scalar("example/no-equality-code", 1, ScalarRepr::String)
            .with_properties(TypeProperties::none())
    }
}

impl DataValue for NoEqualityCode {
    fn datum_ref(&self) -> DatumRef<'_> {
        DatumRef::Value(ValueRef::String(&self.0))
    }
}

#[derive(dol::Model)]
#[dol(key = "example/pipeline-user", name = "PipelineUser")]
struct PipelineUser {
    #[dol(identity)]
    id: u64,
    name: String,
    active: bool,
    age: u16,
    opaque: OpaqueCode,
}

#[derive(dol::Model)]
#[dol(key = "example/no-equality-row", name = "NoEqualityRow")]
struct NoEqualityRow {
    #[dol(identity)]
    id: u64,
    code: NoEqualityCode,
}

#[derive(dol::Model)]
#[dol(key = "example/pipeline-order", name = "PipelineOrder")]
struct PipelineOrder {
    #[dol(identity)]
    id: u64,
    user_id: u64,
}

#[test]
fn pipeline_is_the_declarative_data_computation_abstraction() {
    let pipeline = Pipeline::<PipelineUser>::from_model()
        .filter(PipelineUser::age.ge(18).and(PipelineUser::active.eq(true)))
        .order_by_desc(PipelineUser::age)
        .distinct()
        .limit(10);

    assert_eq!(pipeline.stage_count(), 5);

    let plan = pipeline.logical_plan().unwrap();
    assert_eq!(plan.nodes().len(), 5);
    assert_eq!(plan.root().index(), 4);

    assert!(matches!(plan.nodes()[0], LogicalNode::Source { .. }));
    assert!(matches!(plan.nodes()[1], LogicalNode::Filter { .. }));
    match &plan.nodes()[2] {
        LogicalNode::Sort { keys, .. } => {
            assert_eq!(keys.len(), 1);
            assert_eq!(keys[0].direction(), SortDirection::Descending);
            assert_eq!(keys[0].expression().type_def(), &u16::type_def());
        }
        other => panic!("expected sort node, got {other:?}"),
    }
    assert!(matches!(plan.nodes()[3], LogicalNode::Distinct { .. }));
    assert!(matches!(
        plan.nodes()[4],
        LogicalNode::Slice {
            offset: 0,
            limit: Some(10),
            ..
        }
    ));

    assert!(matches!(
        plan.output(),
        PlanOutput::Model(model) if model.key().as_str() == "example/pipeline-user"
    ));
}

#[test]
fn projection_changes_the_pipeline_output_type_without_creating_a_query_type() {
    let pipeline: Pipeline<String> = Pipeline::<PipelineUser>::from_model()
        .filter(PipelineUser::active.eq(true))
        .order_by(PipelineUser::name)
        .select(PipelineUser::name.trim());

    let plan = pipeline.logical_plan().unwrap();
    assert_eq!(plan.nodes().len(), 4);
    match plan.output() {
        PlanOutput::Value(ty) => assert_eq!(ty, &String::type_def()),
        other => panic!("expected value projection, got {other:?}"),
    }

    match plan.nodes().last().unwrap() {
        LogicalNode::Project { projection, .. } => {
            assert_eq!(projection.kind(), ProjectionKind::Value);
            assert_eq!(projection.type_def(), &String::type_def());
            assert_eq!(projection.expressions().len(), 1);
            assert!(projection.expressions()[0].node_count() >= 1);
            assert_eq!(projection.expressions()[0].scope_count(), 1);
        }
        other => panic!("expected project node, got {other:?}"),
    }
}

#[test]
fn pipeline_fingerprints_are_stable_and_include_logical_operations() {
    let build = || {
        Pipeline::<PipelineUser>::from_model()
            .filter(PipelineUser::active.eq(true))
            .order_by(PipelineUser::age)
            .limit(25)
    };

    let first = build().fingerprint().unwrap();
    let second = build().fingerprint().unwrap();
    let different_limit = Pipeline::<PipelineUser>::from_model()
        .filter(PipelineUser::active.eq(true))
        .order_by(PipelineUser::age)
        .limit(50)
        .fingerprint()
        .unwrap();

    assert_eq!(first, second);
    assert_ne!(first, different_limit);
}

#[test]
fn pipeline_planning_rejects_fields_outside_the_source_scope() {
    let error = Pipeline::<PipelineUser>::from_model()
        .filter(PipelineOrder::user_id.eq(7))
        .logical_plan()
        .unwrap_err();

    assert_eq!(error.code(), "EXPR-SCOPE-001");
}

#[test]
fn projection_closes_the_original_model_scope() {
    let error = Pipeline::<PipelineUser>::from_model()
        .select(PipelineUser::name)
        .filter(PipelineUser::active.eq(true))
        .logical_plan()
        .unwrap_err();

    assert_eq!(error.code(), "EXPR-SCOPE-001");
}

#[test]
fn order_and_distinct_require_semantic_type_capabilities() {
    let order_error = Pipeline::<PipelineUser>::from_model()
        .order_by(PipelineUser::opaque)
        .logical_plan()
        .unwrap_err();
    assert_eq!(order_error.code(), "PIPELINE-TYPE-001");

    let distinct_error = Pipeline::<NoEqualityRow>::from_model()
        .distinct()
        .logical_plan()
        .unwrap_err();
    assert_eq!(distinct_error.code(), "PIPELINE-TYPE-002");

    let projected_distinct_error = Pipeline::<NoEqualityRow>::from_model()
        .select(NoEqualityRow::code)
        .distinct()
        .logical_plan()
        .unwrap_err();
    assert_eq!(projected_distinct_error.code(), "PIPELINE-TYPE-002");
}

#[test]
fn slice_records_offset_and_limit_semantically() {
    let plan = Pipeline::<PipelineUser>::from_model()
        .slice(20, 15)
        .logical_plan()
        .unwrap();

    assert!(matches!(
        plan.nodes().last().unwrap(),
        LogicalNode::Slice {
            offset: 20,
            limit: Some(15),
            ..
        }
    ));
}

#[test]
fn pipeline_limits_are_checked_before_plan_lowering() {
    let pipeline = Pipeline::<PipelineUser>::from_model()
        .filter(PipelineUser::active.eq(true))
        .limit(10);
    let limits = dol::core::limits::PipelineLimits {
        max_nodes: 2,
        ..Default::default()
    };

    let error = pipeline.logical_plan_with_limits(limits).unwrap_err();
    assert_eq!(error.code(), "PIPELINE-LIMIT-002");
}
