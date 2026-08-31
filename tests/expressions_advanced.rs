use std::sync::Arc;

use dol::core::expr::{BindContext, EvalContext, Expr, NumericExprExt};
use dol::core::model::{Model, ModelBuilder, Presence};
use dol::core::runtime::DynRow;
use dol::core::semantics::Truth;
use dol::core::types::DataType;
use dol::core::value::{Datum, Value};

#[derive(dol::Model)]
#[dol(key = "example/advanced-row", name = "AdvancedRow")]
struct AdvancedRow {
    id: u64,
    age: u16,
    small: u8,
    signed: i16,
    score: Option<u8>,
    nickname: Option<String>,
    active: bool,
}

fn row() -> AdvancedRow {
    AdvancedRow {
        id: 7,
        age: 30,
        small: 12,
        signed: -9,
        score: Some(4),
        nickname: Some("Ada".to_owned()),
        active: true,
    }
}

#[test]
fn null_safe_equality_distinguishes_missing_null_and_values() {
    let model = Arc::new(
        ModelBuilder::with_key("example/dynamic-advanced", "DynamicAdvanced")
            .field_def(
                "left",
                "left",
                String::type_def().nullable(),
                Presence::Optional,
            )
            .field_def(
                "right",
                "right",
                String::type_def().nullable(),
                Presence::Optional,
            )
            .freeze()
            .unwrap(),
    );
    let left = model.runtime_field::<Option<String>>("left").unwrap();
    let right = model.runtime_field::<Option<String>>("right").unwrap();

    let both_missing = DynRow::builder(Arc::clone(&model)).freeze().unwrap();
    assert_eq!(
        left.is_not_distinct_from(&right)
            .evaluate_truth(&both_missing)
            .unwrap(),
        Truth::True
    );

    let missing_and_null = DynRow::builder(Arc::clone(&model))
        .set("right", Datum::Null)
        .unwrap()
        .freeze()
        .unwrap();
    assert_eq!(
        left.is_distinct_from(&right)
            .evaluate_truth(&missing_and_null)
            .unwrap(),
        Truth::True
    );
    assert_eq!(
        left.eq(&right).evaluate_truth(&missing_and_null).unwrap(),
        Truth::Unknown
    );
}

#[test]
fn membership_has_three_valued_semantics_and_canonical_candidate_order() {
    let value = row();
    assert_eq!(
        AdvancedRow::age
            .is_in([18_u16, 30_u16, 65_u16])
            .evaluate_truth(&value)
            .unwrap(),
        Truth::True
    );
    assert_eq!(
        AdvancedRow::age
            .not_in([18_u16, 30_u16, 65_u16])
            .evaluate_truth(&value)
            .unwrap(),
        Truth::False
    );
    assert_eq!(
        AdvancedRow::age
            .is_in(std::iter::empty::<u16>())
            .evaluate_truth(&value)
            .unwrap(),
        Truth::False
    );

    let null_nickname = AdvancedRow {
        nickname: None,
        ..row()
    };
    assert_eq!(
        AdvancedRow::nickname
            .is_in([Some("Ada".to_owned()), None])
            .evaluate_truth(&null_nickname)
            .unwrap(),
        Truth::Unknown
    );

    let first = AdvancedRow::age.is_in([18_u16, 30_u16, 65_u16]);
    let reordered = AdvancedRow::age.is_in([65_u16, 18_u16, 30_u16]);
    assert_eq!(
        first.fingerprint().unwrap(),
        reordered.fingerprint().unwrap()
    );
}

#[test]
fn between_is_composition_over_ordered_truth_expressions() {
    let value = row();
    assert_eq!(
        AdvancedRow::age
            .between(18_u16, 65_u16)
            .evaluate_truth(&value)
            .unwrap(),
        Truth::True
    );
    assert_eq!(
        AdvancedRow::age
            .not_between(18_u16, 65_u16)
            .evaluate_truth(&value)
            .unwrap(),
        Truth::False
    );
}

#[test]
fn coalesce_treats_missing_and_null_as_absent_without_erasing_value_semantics() {
    let present = row();
    assert_eq!(
        AdvancedRow::nickname
            .coalesce("unknown")
            .evaluate_datum(&present)
            .unwrap(),
        Datum::Value(Value::String("Ada".to_owned()))
    );

    let null = AdvancedRow {
        nickname: None,
        ..row()
    };
    assert_eq!(
        AdvancedRow::nickname
            .coalesce("unknown")
            .evaluate_datum(&null)
            .unwrap(),
        Datum::Value(Value::String("unknown".to_owned()))
    );

    let model = Arc::new(
        ModelBuilder::with_key("example/coalesce-missing", "CoalesceMissing")
            .field_def(
                "name",
                "name",
                String::type_def().nullable(),
                Presence::Optional,
            )
            .freeze()
            .unwrap(),
    );
    let missing = DynRow::builder(Arc::clone(&model)).freeze().unwrap();
    let name = model.runtime_field::<Option<String>>("name").unwrap();
    assert_eq!(
        name.coalesce("fallback").evaluate_datum(&missing).unwrap(),
        Datum::Value(Value::String("fallback".to_owned()))
    );
}

#[test]
fn conditional_unknown_uses_else_branch_and_unselected_branch_is_not_evaluated() {
    let active = row();
    let expression = AdvancedRow::active
        .eq(true)
        .if_else(1_u16, AdvancedRow::age / 0_u16);
    assert_eq!(
        expression.evaluate_datum(&active).unwrap(),
        Datum::Value(Value::UInt(1))
    );

    let null_score = AdvancedRow {
        score: None,
        ..row()
    };
    let unknown_condition = AdvancedRow::score.gt(0_u8);
    assert_eq!(
        unknown_condition
            .if_else(10_u16, 20_u16)
            .evaluate_datum(&null_score)
            .unwrap(),
        Datum::Value(Value::UInt(20))
    );
}

#[test]
fn conditional_type_inference_is_anchored_by_the_true_branch() {
    let exact = Expr::literal(Truth::True).if_else(7_u16, 9_u16);
    assert_eq!(
        exact.evaluate_datum(&row()).unwrap(),
        Datum::Value(Value::UInt(7))
    );

    let nullable = Expr::literal(Truth::False).if_else(Some(7_u16), 9_u16);
    assert_eq!(
        nullable.evaluate_datum(&row()).unwrap(),
        Datum::Value(Value::UInt(9))
    );
}

#[test]
fn constant_condition_normalization_does_not_evaluate_the_unselected_branch() {
    let invalid_if_evaluated = Expr::literal(10_i16) / 0_i16;
    let expression = Expr::literal(Truth::True).if_else(7_i16, invalid_if_evaluated);
    let prepared = expression.prepare(&BindContext::new()).unwrap();
    assert_eq!(
        prepared.evaluate_datum(&EvalContext::new()).unwrap(),
        Datum::Value(Value::Int(7))
    );
}

#[test]
fn integral_remainder_and_numeric_negation_have_checked_semantics() {
    let value = row();
    assert_eq!(
        (AdvancedRow::age % 7_u16).evaluate_datum(&value).unwrap(),
        Datum::Value(Value::UInt(2))
    );
    assert_eq!(
        (-AdvancedRow::signed).evaluate_datum(&value).unwrap(),
        Datum::Value(Value::Int(9))
    );
}

#[test]
fn explicit_numeric_casts_are_lossless_only_and_preserve_nullability() {
    let value = row();
    assert_eq!(
        AdvancedRow::small
            .cast::<u16>()
            .evaluate_datum(&value)
            .unwrap(),
        Datum::Value(Value::UInt(12))
    );
    assert_eq!(
        AdvancedRow::small
            .cast::<i16>()
            .evaluate_datum(&value)
            .unwrap(),
        Datum::Value(Value::Int(12))
    );
    assert!(
        AdvancedRow::age
            .cast::<u8>()
            .prepare_for(AdvancedRow::model_def().unwrap())
            .is_err()
    );

    let nullable = AdvancedRow::score.cast::<Option<u16>>();
    let null = AdvancedRow {
        score: None,
        ..row()
    };
    assert_eq!(nullable.evaluate_datum(&null).unwrap(), Datum::Null);

    assert!(
        AdvancedRow::score
            .cast::<u16>()
            .prepare_for(AdvancedRow::model_def().unwrap())
            .is_err()
    );
}

#[test]
fn prepared_conditional_uses_same_bound_scope_and_parameter_contracts() {
    let model = AdvancedRow::model_def().unwrap();
    let expression = AdvancedRow::active
        .eq(true)
        .if_else(AdvancedRow::age, 0_u16);
    let prepared = expression.prepare(&BindContext::single(model)).unwrap();
    assert_eq!(
        prepared
            .evaluate_datum(&EvalContext::single(&row()))
            .unwrap(),
        Datum::Value(Value::UInt(30))
    );
}
