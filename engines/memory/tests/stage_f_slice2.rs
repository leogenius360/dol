use dol::core::value::{Datum, Value};
use dol::prelude::*;
use dol_conformance::semantic;
use dol_engine::{ExecutionLimits, ExecutionOptions, ExecutionRow};
use dol_memory::MemoryEngine;

#[derive(Clone, Debug, PartialEq, dol::Model)]
#[dol(key = "stage-f2/user", name = "StageF2User")]
struct User {
    #[dol(identity)]
    id: u64,
    organization_id: u64,
    score: i64,
}

#[derive(Clone, Debug, PartialEq, dol::Model)]
#[dol(key = "stage-f2/order", name = "StageF2Order")]
struct Order {
    #[dol(identity)]
    id: u64,
    user_id: u64,
    total: i64,
}

#[derive(Clone, Debug, PartialEq, dol::Model)]
#[dol(key = "stage-f2/tiny", name = "StageF2Tiny")]
struct Tiny {
    #[dol(identity)]
    id: u64,
    group: bool,
    value: i8,
}

#[derive(Clone, Debug, PartialEq, dol::Model)]
#[dol(key = "stage-f2/float", name = "StageF2Float")]
struct FloatRow {
    #[dol(identity)]
    id: u64,
    value: f64,
}

#[derive(Clone, Debug, PartialEq, dol::Model)]
#[dol(key = "stage-f2/nullable", name = "StageF2Nullable")]
struct NullableRow {
    #[dol(identity)]
    id: u64,
    value: Option<i64>,
}

fn users() -> DataSet<User> {
    DataSet::try_new([
        User {
            id: 1,
            organization_id: 1,
            score: 10,
        },
        User {
            id: 2,
            organization_id: 1,
            score: 20,
        },
        User {
            id: 3,
            organization_id: 1,
            score: 20,
        },
        User {
            id: 4,
            organization_id: 2,
            score: 5,
        },
    ])
    .unwrap()
}

fn orders() -> DataSet<Order> {
    DataSet::try_new([
        Order {
            id: 10,
            user_id: 1,
            total: 50,
        },
        Order {
            id: 11,
            user_id: 1,
            total: 200,
        },
        Order {
            id: 12,
            user_id: 2,
            total: 150,
        },
        Order {
            id: 13,
            user_id: 99,
            total: 500,
        },
    ])
    .unwrap()
}

fn collect<T>(engine: &MemoryEngine, pipeline: &Pipeline<T>) -> Vec<ExecutionRow> {
    let plan = pipeline.logical_plan().unwrap();
    let mut stream = engine
        .execute(pipeline, &Parameters::new(), &ExecutionOptions::default())
        .unwrap();
    semantic::collect_checked(&mut stream, plan.output())
        .unwrap()
        .into_vec()
}

fn values(rows: &[ExecutionRow]) -> Vec<Datum> {
    rows.iter()
        .map(|row| match row {
            ExecutionRow::Value(value) => value.clone(),
            other => panic!("expected value execution row, got {other:?}"),
        })
        .collect()
}

#[test]
fn inner_and_outer_joins_materialize_exact_products_and_null_scopes() {
    let mut engine = MemoryEngine::new();
    engine.load(&users()).unwrap();
    engine.load(&orders()).unwrap();

    let inner = Pipeline::<User>::from_model()
        .join(Pipeline::<Order>::from_model(), User::id.eq(Order::user_id))
        .select((User::id, Order::id));
    assert_eq!(
        values(&collect(&engine, &inner)),
        vec![
            Datum::Value(Value::Tuple(vec![
                Datum::Value(Value::UInt(1)),
                Datum::Value(Value::UInt(10)),
            ])),
            Datum::Value(Value::Tuple(vec![
                Datum::Value(Value::UInt(1)),
                Datum::Value(Value::UInt(11)),
            ])),
            Datum::Value(Value::Tuple(vec![
                Datum::Value(Value::UInt(2)),
                Datum::Value(Value::UInt(12)),
            ])),
        ]
    );

    let left = Pipeline::<User>::from_model()
        .left_join(Pipeline::<Order>::from_model(), User::id.eq(Order::user_id))
        .select((User::id, Order::id.nullable()));
    let left_values = values(&collect(&engine, &left));
    assert_eq!(left_values.len(), 5);
    assert_eq!(
        left_values[3],
        Datum::Value(Value::Tuple(vec![
            Datum::Value(Value::UInt(3)),
            Datum::Null,
        ]))
    );
    assert_eq!(
        left_values[4],
        Datum::Value(Value::Tuple(vec![
            Datum::Value(Value::UInt(4)),
            Datum::Null,
        ]))
    );

    let full = Pipeline::<User>::from_model()
        .full_join(Pipeline::<Order>::from_model(), User::id.eq(Order::user_id))
        .select((User::id.nullable(), Order::id.nullable()));
    let full_values = values(&collect(&engine, &full));
    assert!(full_values.contains(&Datum::Value(Value::Tuple(vec![
        Datum::Null,
        Datum::Value(Value::UInt(13)),
    ]))));
}

#[test]
fn cross_join_limit_is_rejected_before_product_materialization() {
    let mut engine = MemoryEngine::new();
    engine.load(&users()).unwrap();
    engine.load(&orders()).unwrap();
    let pipeline = Pipeline::<User>::from_model().cross_join(Pipeline::<Order>::from_model());
    let options = ExecutionOptions {
        limits: ExecutionLimits {
            max_materialized_rows: Some(8),
            ..ExecutionLimits::default()
        },
        ..ExecutionOptions::default()
    };
    let error = engine
        .execute(&pipeline, &Parameters::new(), &options)
        .unwrap_err();
    assert_eq!(error.code(), "ENGINE-LIMIT-004");
}

#[test]
fn set_algebra_and_distinct_follow_dol_equality() {
    let mut engine = MemoryEngine::new();
    engine.load(&users()).unwrap();

    let left = Pipeline::<User>::from_model()
        .filter(User::id.lt(3))
        .select(User::id);
    let right = Pipeline::<User>::from_model()
        .filter(User::id.gt(1).and(User::id.lt(4)))
        .select(User::id);

    assert_eq!(
        values(&collect(&engine, &left.clone().union(right.clone()))),
        vec![
            Datum::Value(Value::UInt(1)),
            Datum::Value(Value::UInt(2)),
            Datum::Value(Value::UInt(3)),
        ]
    );
    assert_eq!(
        values(&collect(&engine, &left.clone().union_all(right.clone()))),
        vec![
            Datum::Value(Value::UInt(1)),
            Datum::Value(Value::UInt(2)),
            Datum::Value(Value::UInt(2)),
            Datum::Value(Value::UInt(3)),
        ]
    );
    assert_eq!(
        values(&collect(&engine, &left.clone().intersect(right.clone()))),
        vec![Datum::Value(Value::UInt(2))]
    );
    assert_eq!(
        values(&collect(&engine, &left.clone().except(right.clone()))),
        vec![Datum::Value(Value::UInt(1))]
    );
    assert_eq!(
        values(&collect(&engine, &left.union_all(right).distinct(),)),
        vec![
            Datum::Value(Value::UInt(1)),
            Datum::Value(Value::UInt(2)),
            Datum::Value(Value::UInt(3)),
        ]
    );
}

#[test]
fn sorting_has_explicit_null_and_ieee_total_float_order() {
    let floats = DataSet::try_new([
        FloatRow {
            id: 1,
            value: f64::NAN,
        },
        FloatRow { id: 2, value: 0.0 },
        FloatRow { id: 3, value: -0.0 },
        FloatRow {
            id: 4,
            value: f64::NEG_INFINITY,
        },
        FloatRow {
            id: 5,
            value: f64::INFINITY,
        },
    ])
    .unwrap();
    let nullable = DataSet::try_new([
        NullableRow {
            id: 1,
            value: Some(2),
        },
        NullableRow { id: 2, value: None },
        NullableRow {
            id: 3,
            value: Some(1),
        },
    ])
    .unwrap();
    let mut engine = MemoryEngine::new();
    engine.load(&floats).unwrap();
    engine.load(&nullable).unwrap();

    let sorted = Pipeline::<FloatRow>::from_model()
        .order_by(FloatRow::value)
        .select(FloatRow::value);
    let sorted = values(&collect(&engine, &sorted));
    let bits = sorted
        .iter()
        .map(|datum| match datum {
            Datum::Value(Value::Float64(value)) => value.to_bits(),
            other => panic!("expected f64 datum, got {other:?}"),
        })
        .collect::<Vec<_>>();
    assert_eq!(
        bits,
        vec![
            f64::NEG_INFINITY.to_bits(),
            (-0.0_f64).to_bits(),
            0.0_f64.to_bits(),
            f64::INFINITY.to_bits(),
            f64::NAN.to_bits(),
        ]
    );

    let nullable_sorted = Pipeline::<NullableRow>::from_model()
        .order_by(NullableRow::value)
        .select(NullableRow::value);
    assert_eq!(
        values(&collect(&engine, &nullable_sorted)),
        vec![
            Datum::Null,
            Datum::Value(Value::Int(1)),
            Datum::Value(Value::Int(2)),
        ]
    );
}

#[test]
fn aggregates_use_order_independent_exact_sum_and_group_semantics() {
    let data = DataSet::try_new([
        Tiny {
            id: 1,
            group: true,
            value: 120,
        },
        Tiny {
            id: 2,
            group: true,
            value: 120,
        },
        Tiny {
            id: 3,
            group: true,
            value: -120,
        },
        Tiny {
            id: 4,
            group: false,
            value: 5,
        },
    ])
    .unwrap();
    let mut engine = MemoryEngine::new();
    engine.load(&data).unwrap();

    let global = Pipeline::<Tiny>::from_model()
        .filter(Tiny::group.eq(true))
        .aggregate((
            count(),
            sum(Tiny::value),
            min(Tiny::value),
            max(Tiny::value),
        ));
    assert_eq!(
        values(&collect(&engine, &global)),
        vec![Datum::Value(Value::Tuple(vec![
            Datum::Value(Value::UInt(3)),
            Datum::Value(Value::Int(120)),
            Datum::Value(Value::Int(-120)),
            Datum::Value(Value::Int(120)),
        ]))]
    );

    let grouped =
        Pipeline::<Tiny>::from_model().aggregate_by(Tiny::group, (count(), sum(Tiny::value)));
    let grouped = values(&collect(&engine, &grouped));
    assert_eq!(grouped.len(), 2);
    assert!(grouped.contains(&Datum::Value(Value::Tuple(vec![
        Datum::Value(Value::Bool(true)),
        Datum::Value(Value::Tuple(vec![
            Datum::Value(Value::UInt(3)),
            Datum::Value(Value::Int(120)),
        ])),
    ]))));
}

#[test]
fn ranking_and_rows_windows_preserve_input_rows_and_exact_frames() {
    let mut engine = MemoryEngine::new();
    engine.load(&users()).unwrap();

    let rank_pipeline = Pipeline::<User>::from_model().window(
        rank()
            .partition_by(User::organization_id)
            .order_by_desc(User::score),
    );
    assert_eq!(
        window_u64(&collect(&engine, &rank_pipeline)),
        vec![3, 1, 1, 1]
    );

    let dense_pipeline = Pipeline::<User>::from_model().window(
        dense_rank()
            .partition_by(User::organization_id)
            .order_by_desc(User::score),
    );
    assert_eq!(
        window_u64(&collect(&engine, &dense_pipeline)),
        vec![2, 1, 1, 1]
    );

    let row_numbers = Pipeline::<User>::from_model()
        .window(row_number().order_by(User::score).order_by(User::id));
    assert_eq!(
        window_u64(&collect(&engine, &row_numbers)),
        vec![2, 3, 4, 1]
    );

    let running = Pipeline::<User>::from_model().window(
        window_sum(User::score)
            .partition_by(User::organization_id)
            .order_by(User::id)
            .rows(RowsFrame::to_current()),
    );
    assert_eq!(window_i64(&collect(&engine, &running)), vec![10, 30, 50, 5]);
}

fn window_u64(rows: &[ExecutionRow]) -> Vec<u64> {
    rows.iter()
        .map(|row| match row {
            ExecutionRow::Product(parts) => match parts.get(1) {
                Some(ExecutionRow::Value(Datum::Value(Value::UInt(value)))) => {
                    u64::try_from(*value).unwrap()
                }
                other => panic!("expected u64 window value, got {other:?}"),
            },
            other => panic!("expected product row, got {other:?}"),
        })
        .collect()
}

fn window_i64(rows: &[ExecutionRow]) -> Vec<i64> {
    rows.iter()
        .map(|row| match row {
            ExecutionRow::Product(parts) => match parts.get(1) {
                Some(ExecutionRow::Value(Datum::Value(Value::Int(value)))) => {
                    i64::try_from(*value).unwrap()
                }
                other => panic!("expected i64 window value, got {other:?}"),
            },
            other => panic!("expected product row, got {other:?}"),
        })
        .collect()
}

#[test]
fn correlated_exists_executes_at_the_exact_filter_position() {
    let mut engine = MemoryEngine::new();
    engine.load(&users()).unwrap();
    engine.load(&orders()).unwrap();

    let has_large_order = Pipeline::<Order>::from_model()
        .filter(Order::user_id.eq(User::id).and(Order::total.gt(100)))
        .exists();
    let users_with_large_orders = Pipeline::<User>::from_model()
        .filter(has_large_order)
        .select(User::id);
    assert_eq!(
        values(&collect(&engine, &users_with_large_orders)),
        vec![Datum::Value(Value::UInt(1)), Datum::Value(Value::UInt(2)),]
    );

    let position_sensitive = Pipeline::<Order>::from_model()
        .filter(Order::user_id.eq(User::id))
        .limit(1)
        .filter(Order::total.gt(100))
        .exists();
    let selected = Pipeline::<User>::from_model()
        .filter(position_sensitive)
        .select(User::id);
    assert_eq!(
        values(&collect(&engine, &selected)),
        vec![Datum::Value(Value::UInt(2))]
    );

    let no_orders = Pipeline::<Order>::from_model()
        .filter(Order::user_id.eq(User::id))
        .not_exists();
    let without_orders = Pipeline::<User>::from_model()
        .filter(no_orders)
        .select(User::id);
    assert_eq!(
        values(&collect(&engine, &without_orders)),
        vec![Datum::Value(Value::UInt(3)), Datum::Value(Value::UInt(4)),]
    );
}
