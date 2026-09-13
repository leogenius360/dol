use dol::core::plan::{JoinKind, LogicalNode, PlanOutput, SetOperator};
use dol::core::types::{DataType, ScalarRepr, TypeDef, TypeProperties};
use dol::core::value::{DataValue, DatumRef, ValueRef};
use dol::prelude::*;

#[derive(dol::Model)]
#[dol(key = "example/employee", name = "Employee")]
struct Employee {
    #[dol(identity)]
    id: u64,
    manager_id: u64,
    department_id: u64,
    active: bool,
}

#[derive(dol::Model)]
#[dol(key = "example/department", name = "Department")]
struct Department {
    #[dol(identity)]
    id: u64,
    name: String,
}

#[derive(Clone, Debug)]
struct NoEquality(String);

impl DataType for NoEquality {
    fn type_def() -> TypeDef {
        TypeDef::scalar("example/no-equality-set", 1, ScalarRepr::String)
            .with_properties(TypeProperties::none())
    }
}

impl DataValue for NoEquality {
    fn datum_ref(&self) -> DatumRef<'_> {
        DatumRef::Value(ValueRef::String(&self.0))
    }
}

#[derive(dol::Model)]
#[dol(key = "example/set-row", name = "SetRow")]
struct SetRow {
    #[dol(identity)]
    id: u64,
    value: NoEquality,
}

#[test]
fn inner_join_is_one_pipeline_over_a_multi_source_dag() {
    let pipeline: Pipeline<(Employee, Department)> = Pipeline::<Employee>::from_model().join(
        Pipeline::<Department>::from_model(),
        Employee::department_id.eq(Department::id),
    );

    assert_eq!(pipeline.stage_count(), 3);
    let plan = pipeline.logical_plan().unwrap();
    assert_eq!(plan.nodes().len(), 3);
    match plan.nodes().last().unwrap() {
        LogicalNode::Join {
            kind,
            condition: Some(condition),
            ..
        } => {
            assert_eq!(*kind, JoinKind::Inner);
            assert_eq!(condition.scope_count(), 2);
        }
        other => panic!("expected inner join, got {other:?}"),
    }
    match plan.output() {
        PlanOutput::Product(outputs) => {
            assert_eq!(outputs.len(), 2);
            assert!(
                matches!(&outputs[0], PlanOutput::Model(model) if model.key().as_str() == "example/employee")
            );
            assert!(
                matches!(&outputs[1], PlanOutput::Model(model) if model.key().as_str() == "example/department")
            );
        }
        other => panic!("expected product output, got {other:?}"),
    }
}

#[test]
fn joined_scopes_remain_available_until_projection() {
    let names: Pipeline<String> = Pipeline::<Employee>::from_model()
        .join(
            Pipeline::<Department>::from_model(),
            Employee::department_id.eq(Department::id),
        )
        .filter(Employee::active.eq(true))
        .select(Department::name.trim());

    let plan = names.logical_plan().unwrap();
    assert_eq!(plan.nodes().len(), 5);
    assert!(matches!(plan.output(), PlanOutput::Value(ty) if ty == &String::type_def()));
}

#[test]
fn self_join_uses_explicit_source_aliases_without_changing_pipeline_identity() {
    let build = |employee: &'static str, manager: &'static str| {
        Pipeline::<Employee>::from_model_as(employee)
            .join(
                Pipeline::<Employee>::from_model_as(manager),
                Employee::manager_id
                    .at(employee)
                    .eq(Employee::id.at(manager)),
            )
            .select(Employee::id.at(manager))
    };

    let descriptive = build("employee", "manager");
    let short = build("e", "m");
    assert_eq!(
        descriptive.fingerprint().unwrap(),
        short.fingerprint().unwrap()
    );

    let plan = descriptive.logical_plan().unwrap();
    match &plan.nodes()[2] {
        LogicalNode::Join {
            condition: Some(condition),
            ..
        } => assert_eq!(condition.scope_count(), 2),
        other => panic!("expected self join, got {other:?}"),
    }
}

#[test]
fn duplicate_model_sources_require_aliases_and_aliases_must_be_unique() {
    let missing_alias = Pipeline::<Employee>::from_model()
        .join(
            Pipeline::<Employee>::from_model(),
            Employee::manager_id.eq(Employee::id),
        )
        .logical_plan()
        .unwrap_err();
    assert_eq!(missing_alias.code(), "PIPELINE-SCOPE-001");

    let duplicate_alias = Pipeline::<Employee>::from_model_as("row")
        .join(
            Pipeline::<Department>::from_model_as("row"),
            Employee::department_id
                .at("row")
                .eq(Department::id.at("row")),
        )
        .logical_plan()
        .unwrap_err();
    assert_eq!(duplicate_alias.code(), "PIPELINE-SCOPE-002");

    let invalid_alias = Pipeline::<Employee>::from_model_as("not valid")
        .logical_plan()
        .unwrap_err();
    assert_eq!(invalid_alias.code(), "PIPELINE-SCOPE-003");
}

#[test]
fn scoped_field_reports_unknown_alias_at_expression_binding() {
    let error = Pipeline::<Employee>::from_model_as("employee")
        .filter(Employee::active.at("other").eq(true))
        .logical_plan()
        .unwrap_err();
    assert_eq!(error.code(), "EXPR-SCOPE-005");
}

#[test]
fn cross_join_has_no_condition_and_preserves_both_outputs() {
    let plan = Pipeline::<Employee>::from_model()
        .cross_join(Pipeline::<Department>::from_model())
        .logical_plan()
        .unwrap();

    match plan.nodes().last().unwrap() {
        LogicalNode::Join {
            kind, condition, ..
        } => {
            assert_eq!(*kind, JoinKind::Cross);
            assert!(condition.is_none());
        }
        other => panic!("expected cross join, got {other:?}"),
    }
}

#[test]
fn set_operations_share_subgraphs_and_keep_model_scope() {
    let source = Pipeline::<Employee>::from_model();
    let active = source.clone().filter(Employee::active.eq(true));
    let managed = source.filter(Employee::manager_id.gt(0));

    let pipeline = active.union(managed).filter(Employee::department_id.gt(0));

    assert_eq!(pipeline.stage_count(), 5);
    let plan = pipeline.logical_plan().unwrap();
    assert_eq!(plan.nodes().len(), 5);
    assert!(matches!(
        &plan.nodes()[3],
        LogicalNode::Set {
            operator: SetOperator::Union,
            ..
        }
    ));
    assert!(matches!(
        plan.nodes().last().unwrap(),
        LogicalNode::Filter { .. }
    ));
}

#[test]
fn commutative_set_operations_have_canonical_input_identity() {
    let active = Pipeline::<Employee>::from_model().filter(Employee::active.eq(true));
    let managed = Pipeline::<Employee>::from_model().filter(Employee::manager_id.gt(0));

    assert_eq!(
        active.clone().union(managed.clone()).fingerprint().unwrap(),
        managed.clone().union(active.clone()).fingerprint().unwrap()
    );
    assert_eq!(
        active
            .clone()
            .intersect(managed.clone())
            .fingerprint()
            .unwrap(),
        managed
            .clone()
            .intersect(active.clone())
            .fingerprint()
            .unwrap()
    );
    assert_ne!(
        active
            .clone()
            .except(managed.clone())
            .fingerprint()
            .unwrap(),
        managed.except(active).fingerprint().unwrap()
    );
}

#[test]
fn union_all_does_not_require_equality_but_set_semantics_do() {
    Pipeline::<SetRow>::from_model()
        .union_all(Pipeline::<SetRow>::from_model())
        .logical_plan()
        .unwrap();

    let error = Pipeline::<SetRow>::from_model()
        .union(Pipeline::<SetRow>::from_model())
        .logical_plan()
        .unwrap_err();
    assert_eq!(error.code(), "PIPELINE-TYPE-002");
}

#[test]
fn scoped_fields_keep_their_type_owned_symbolic_api() {
    let pipeline: Pipeline<String> = Pipeline::<Department>::from_model_as("department")
        .select(Department::name.at("department").trim().to_uppercase());

    let plan = pipeline.logical_plan().unwrap();
    assert!(matches!(plan.output(), PlanOutput::Value(ty) if ty == &String::type_def()));
}
