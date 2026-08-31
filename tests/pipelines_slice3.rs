use dol::core::analytics::{AggregateKind, AggregateSelectionKind};
use dol::core::plan::{JoinKind, LogicalNode, PlanOutput, ProjectionKind};
use dol::core::types::{DataType, ScalarRepr, TypeDef, TypeProperties, TypeShape};
use dol::core::value::{DataValue, DatumRef, ValueRef};
use dol::prelude::*;

#[derive(dol::Model)]
#[dol(key = "example/slice3-employee", name = "Slice3Employee")]
struct Employee {
    #[dol(identity)]
    id: u64,
    department_id: u64,
    age: u16,
    name: String,
    active: bool,
    score: f64,
    quota: Option<u64>,
}

#[derive(dol::Model)]
#[dol(key = "example/slice3-department", name = "Slice3Department")]
struct Department {
    #[dol(identity)]
    id: u64,
    name: String,
    note: Option<String>,
}

#[derive(dol::Projection)]
#[dol(key = "example/slice3-summary")]
struct EmployeeSummary {
    name: String,
    id: u64,
}

#[derive(dol::Projection)]
#[dol(key = "example/slice3-summary")]
struct EmployeeSummaryReordered {
    id: u64,
    name: String,
}

#[derive(Clone, Debug)]
struct Unkeyed(String);

impl DataType for Unkeyed {
    fn type_def() -> TypeDef {
        let mut properties = TypeProperties::none();
        properties.equality = true;
        TypeDef::scalar("example/slice3-unkeyed", 1, ScalarRepr::String).with_properties(properties)
    }
}

impl DataValue for Unkeyed {
    fn datum_ref(&self) -> DatumRef<'_> {
        DatumRef::Value(ValueRef::String(&self.0))
    }
}

#[derive(dol::Model)]
#[dol(key = "example/slice3-unkeyed-row", name = "Slice3UnkeyedRow")]
struct UnkeyedRow {
    #[dol(identity)]
    id: u64,
    value: Unkeyed,
}

#[test]
fn tuple_projection_is_one_typed_structural_project_node() {
    let pipeline: Pipeline<(u64, String)> =
        Pipeline::<Employee>::from_model().select((Employee::id, Employee::name.trim()));

    let plan = pipeline.logical_plan().unwrap();
    let LogicalNode::Project { projection, .. } = plan.nodes().last().unwrap() else {
        panic!("expected project node");
    };
    assert_eq!(projection.kind(), ProjectionKind::Tuple);
    assert_eq!(projection.expressions().len(), 2);
    assert!(matches!(projection.type_def().shape(), TypeShape::Tuple(parts) if parts.len() == 2));
}

#[test]
fn named_record_projection_is_canonical_by_logical_field_name() {
    let pipeline: Pipeline<EmployeeSummary> = Pipeline::<Employee>::from_model().select(
        EmployeeSummary::project((Employee::name.trim(), Employee::id)),
    );

    let plan = pipeline.logical_plan().unwrap();
    let LogicalNode::Project { projection, .. } = plan.nodes().last().unwrap() else {
        panic!("expected project node");
    };
    assert_eq!(projection.kind(), ProjectionKind::Record);
    assert_eq!(
        projection
            .names()
            .iter()
            .map(|name| name.as_ref())
            .collect::<Vec<_>>(),
        vec!["id", "name"]
    );
    assert_eq!(projection.type_def(), &EmployeeSummary::type_def());

    let reordered = Pipeline::<Employee>::from_model().select(EmployeeSummaryReordered::project((
        Employee::id,
        Employee::name.trim(),
    )));
    let reordered_plan = reordered.logical_plan().unwrap();
    assert_eq!(plan.fingerprint(), reordered_plan.fingerprint());
}

#[test]
fn global_and_grouped_aggregation_remain_pipeline_operations() {
    let global: Pipeline<(u64, Option<u16>, Option<String>)> = Pipeline::<Employee>::from_model()
        .left_join(
            Pipeline::<Department>::from_model(),
            Employee::department_id.eq(Department::id),
        )
        .aggregate((
            count(),
            sum(Employee::age),
            min_present(Department::name.nullable()),
        ));

    let global_plan = global.logical_plan().unwrap();
    let LogicalNode::Aggregate {
        groups, aggregates, ..
    } = global_plan.nodes().last().unwrap()
    else {
        panic!("expected aggregate node");
    };
    assert!(groups.is_none());
    assert_eq!(aggregates.kind(), AggregateSelectionKind::Tuple);
    assert_eq!(aggregates.aggregates().len(), 3);
    assert_eq!(aggregates.aggregates()[0].kind(), AggregateKind::CountRows);
    assert_eq!(aggregates.aggregates()[1].kind(), AggregateKind::Sum);
    assert_eq!(aggregates.aggregates()[2].kind(), AggregateKind::Min);

    let grouped: Pipeline<(bool, (u64, Option<u16>))> = Pipeline::<Employee>::from_model()
        .aggregate_by(Employee::active, (count(), max(Employee::age)));
    let grouped_plan = grouped.logical_plan().unwrap();
    let LogicalNode::Aggregate { groups, .. } = grouped_plan.nodes().last().unwrap() else {
        panic!("expected grouped aggregate node");
    };
    assert!(groups.is_some());
}

#[test]
fn aggregate_planning_rejects_unlocked_or_ambiguous_semantics() {
    let approximate = Pipeline::<Employee>::from_model()
        .aggregate(sum(Employee::score))
        .logical_plan()
        .unwrap_err();
    assert_eq!(approximate.code(), "PIPELINE-AGG-004");

    let nullable = Pipeline::<Employee>::from_model()
        .aggregate(sum(Employee::quota))
        .logical_plan()
        .unwrap_err();
    assert_eq!(nullable.code(), "PIPELINE-AGG-006");

    Pipeline::<Employee>::from_model()
        .aggregate(sum_present(Employee::quota))
        .logical_plan()
        .unwrap();

    let unkeyed = Pipeline::<UnkeyedRow>::from_model()
        .aggregate_by(UnkeyedRow::value, count())
        .logical_plan()
        .unwrap_err();
    assert_eq!(unkeyed.code(), "PIPELINE-GROUP-001");
}

#[test]
fn outer_joins_lift_plan_output_and_require_explicit_field_nullability() {
    let joined: Pipeline<(Employee, Option<Department>)> = Pipeline::<Employee>::from_model()
        .left_join(
            Pipeline::<Department>::from_model(),
            Employee::department_id.eq(Department::id),
        );
    let plan = joined.logical_plan().unwrap();
    let LogicalNode::Join { kind, .. } = plan.nodes().last().unwrap() else {
        panic!("expected join node");
    };
    assert_eq!(*kind, JoinKind::Left);
    match plan.output() {
        PlanOutput::Product(outputs) => {
            assert!(matches!(&outputs[0], PlanOutput::Model(_)));
            assert!(matches!(
                &outputs[1],
                PlanOutput::Nullable(output)
                    if matches!(output.as_ref(), PlanOutput::Model(_))
            ));
        }
        other => panic!("expected outer product output, got {other:?}"),
    }

    let implicit = joined
        .clone()
        .select(Department::name)
        .logical_plan()
        .unwrap_err();
    assert_eq!(implicit.code(), "EXPR-SCOPE-006");

    let lifted: Pipeline<Option<String>> = joined.clone().select(Department::name.nullable());
    lifted.logical_plan().unwrap();

    // A field that is already nullable does not require a second nullable lift.
    let already_nullable: Pipeline<Option<String>> = joined.select(Department::note);
    already_nullable.logical_plan().unwrap();

    let employee = "employee";
    let manager = "manager";
    let self_join = Pipeline::<Employee>::from_model_as(employee).left_join(
        Pipeline::<Employee>::from_model_as(manager),
        Employee::department_id
            .at(employee)
            .eq(Employee::id.at(manager)),
    );
    let aliased: Pipeline<Option<String>> = self_join.select(Employee::name.at(manager).nullable());
    aliased.logical_plan().unwrap();
}

#[test]
fn right_and_full_joins_lift_the_correct_sides() {
    let right: Pipeline<(Option<Employee>, Department)> = Pipeline::<Employee>::from_model()
        .right_join(
            Pipeline::<Department>::from_model(),
            Employee::department_id.eq(Department::id),
        );
    let full: Pipeline<(Option<Employee>, Option<Department>)> = Pipeline::<Employee>::from_model()
        .full_join(
            Pipeline::<Department>::from_model(),
            Employee::department_id.eq(Department::id),
        );

    let right_plan = right.logical_plan().unwrap();
    let full_plan = full.logical_plan().unwrap();
    assert!(matches!(
        right_plan.output(),
        PlanOutput::Product(outputs)
            if matches!(&outputs[0], PlanOutput::Nullable(_))
                && matches!(&outputs[1], PlanOutput::Model(_))
    ));
    assert!(matches!(
        full_plan.output(),
        PlanOutput::Product(outputs)
            if matches!(&outputs[0], PlanOutput::Nullable(_))
                && matches!(&outputs[1], PlanOutput::Nullable(_))
    ));
    assert_ne!(right_plan.fingerprint(), full_plan.fingerprint());
}
