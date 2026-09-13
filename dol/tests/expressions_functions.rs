use dol::core::expr::{
    BindContext, DateExprExt, DurationExprExt, EvalContext, Expr, InstantExprExt,
    LocalDateTimeExprExt, NullableDateExprExt, NullableStringExprExt, StringExprExt,
};
use dol::core::model::Model;
use dol::core::value::{Datum, Value};

#[derive(dol::Model)]
#[dol(key = "example/function-row", name = "FunctionRow")]
struct FunctionRow {
    text: String,
    optional_text: Option<String>,
    date: time::Date,
    optional_date: Option<time::Date>,
    local: time::PrimitiveDateTime,
    instant: time::OffsetDateTime,
    duration: time::Duration,
}

fn row() -> FunctionRow {
    let date = time::Date::from_calendar_date(2026, time::Month::August, 29).unwrap();
    let clock = time::Time::from_hms(13, 45, 30).unwrap();
    FunctionRow {
        text: "  HéLLo 🦀  ".to_owned(),
        optional_text: Some("Rust".to_owned()),
        date,
        optional_date: Some(date),
        local: time::PrimitiveDateTime::new(date, clock),
        instant: time::OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
        duration: time::Duration::seconds(3_661),
    }
}

#[test]
fn text_functions_have_explicit_scalar_and_unicode_semantics() {
    let value = row();

    assert_eq!(
        FunctionRow::text
            .trim()
            .to_lowercase()
            .evaluate_datum(&value)
            .unwrap(),
        Datum::Value(Value::String("héllo 🦀".to_owned()))
    );
    assert_eq!(
        FunctionRow::text
            .trim()
            .to_uppercase()
            .evaluate_datum(&value)
            .unwrap(),
        Datum::Value(Value::String("HÉLLO 🦀".to_owned()))
    );
    assert_eq!(
        FunctionRow::text.len().evaluate_datum(&value).unwrap(),
        Datum::Value(Value::UInt(15))
    );
    assert_eq!(
        FunctionRow::text
            .trim()
            .scalar_len()
            .evaluate_datum(&value)
            .unwrap(),
        Datum::Value(Value::UInt(7))
    );
    assert_eq!(
        FunctionRow::text
            .contains("HéLLo")
            .evaluate_truth(&value)
            .unwrap(),
        dol::Truth::True
    );
    assert_eq!(
        FunctionRow::text
            .contains("héllo")
            .evaluate_truth(&value)
            .unwrap(),
        dol::Truth::False
    );
}

#[test]
fn temporal_functions_preserve_domain_boundaries() {
    let value = row();

    assert_eq!(
        FunctionRow::date.year().evaluate_datum(&value).unwrap(),
        Datum::Value(Value::Int(2026))
    );
    let month: Expr<time::Month> = FunctionRow::date.month();
    assert_eq!(
        month.evaluate_datum(&value).unwrap(),
        Datum::Value(Value::UInt(8))
    );
    let weekday: Expr<time::Weekday> = FunctionRow::date.weekday();
    assert_eq!(
        weekday.evaluate_datum(&value).unwrap(),
        Datum::Value(Value::UInt(6))
    );
    assert_eq!(
        FunctionRow::date.day().evaluate_datum(&value).unwrap(),
        Datum::Value(Value::UInt(29))
    );
    assert_eq!(
        FunctionRow::local.date().evaluate_datum(&value).unwrap(),
        Datum::Value(Value::Date(value.date))
    );
    assert_eq!(
        FunctionRow::local.time().evaluate_datum(&value).unwrap(),
        Datum::Value(Value::Time(time::Time::from_hms(13, 45, 30).unwrap()))
    );
    assert_eq!(
        FunctionRow::instant
            .unix_timestamp()
            .evaluate_datum(&value)
            .unwrap(),
        Datum::Value(Value::Int(1_700_000_000))
    );
    assert_eq!(
        FunctionRow::duration
            .whole_seconds()
            .evaluate_datum(&value)
            .unwrap(),
        Datum::Value(Value::Int(3_661))
    );
}

#[test]
fn nullable_functions_preserve_null_state() {
    let value = FunctionRow {
        optional_text: None,
        optional_date: None,
        ..row()
    };

    assert_eq!(
        FunctionRow::optional_text
            .scalar_len()
            .evaluate_datum(&value)
            .unwrap(),
        Datum::Null
    );
    assert_eq!(
        FunctionRow::optional_date
            .year()
            .evaluate_datum(&value)
            .unwrap(),
        Datum::Null
    );
}

#[test]
fn deterministic_function_calls_constant_fold_and_fingerprint_stably() {
    let expression = Expr::literal("  Rust 🦀  ".to_owned())
        .trim()
        .to_lowercase()
        .scalar_len();
    let rebuilt = Expr::literal("  Rust 🦀  ".to_owned())
        .trim()
        .to_lowercase()
        .scalar_len();

    assert_eq!(
        expression.fingerprint().unwrap(),
        rebuilt.fingerprint().unwrap()
    );

    let normalized = expression.normalized().unwrap();
    let prepared = normalized.prepare(&BindContext::new()).unwrap();
    assert_eq!(prepared.node_count(), 1);
    assert_eq!(
        prepared.evaluate_datum(&EvalContext::new()).unwrap(),
        Datum::Value(Value::UInt(6))
    );
}

#[test]
fn different_semantic_functions_have_different_expression_identity() {
    let lower = FunctionRow::text.to_lowercase();
    let upper = FunctionRow::text.to_uppercase();
    let length = FunctionRow::text.scalar_len();

    assert_ne!(lower.fingerprint().unwrap(), upper.fingerprint().unwrap());
    assert_ne!(lower.fingerprint().unwrap(), length.fingerprint().unwrap());
}

#[test]
fn normalization_preserves_results_across_expression_families() {
    let value = row();

    assert_equivalent(&(FunctionRow::date.year() + 1_i32), &value);
    assert_equivalent(&FunctionRow::text.trim().scalar_len(), &value);
    assert_equivalent(
        &FunctionRow::text
            .contains("HéLLo")
            .if_else(FunctionRow::date.day(), 0_u8),
        &value,
    );
    assert_equivalent(
        &FunctionRow::date
            .year()
            .is_in([2025_i32, 2026_i32, 2027_i32]),
        &value,
    );
}

fn assert_equivalent<T>(expression: &Expr<T>, row: &FunctionRow) {
    let normalized = expression.normalized().unwrap();
    assert_eq!(
        expression.evaluate_datum(row).unwrap(),
        normalized.evaluate_datum(row).unwrap()
    );
    assert_eq!(
        expression.fingerprint().unwrap(),
        normalized.fingerprint().unwrap()
    );
}

#[test]
fn function_result_types_remain_rust_typed() {
    let model = FunctionRow::model_def().unwrap();
    let expression: Expr<i32> = FunctionRow::date.year();
    let prepared = expression.prepare_for(model).unwrap();
    let value = row();
    assert_eq!(
        prepared
            .evaluate_datum(&EvalContext::single(&value))
            .unwrap(),
        Datum::Value(Value::Int(2026))
    );
}
