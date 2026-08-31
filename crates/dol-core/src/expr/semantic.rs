use core::cmp::Ordering;

use crate::diagnostic::{Diagnostic, Result};
use crate::semantics::Truth;
use crate::types::{Nullability, ScalarRepr, TypeDef, TypeShape};
use crate::value::{Datum, Value};

use super::function::FunctionDef;
use super::node::{BinaryOp, UnaryOp};

pub(crate) fn eval_unary(op: UnaryOp, input: Datum, output: &TypeDef) -> Result<Datum> {
    match op {
        UnaryOp::Not => truth_unary(input, Truth::negate),
        UnaryOp::IsNull => Ok(truth(matches!(input, Datum::Null))),
        UnaryOp::IsMissing => Ok(truth(matches!(input, Datum::Missing))),
        UnaryOp::IsPresent => Ok(truth(!matches!(input, Datum::Missing))),
        UnaryOp::NullableLift => Ok(input),
        UnaryOp::LosslessCast => cast_lossless(input, output),
        UnaryOp::Negate => numeric_negate(input),
        UnaryOp::Abs => numeric_abs(input),
    }
}

pub(crate) fn eval_binary(op: BinaryOp, left: Datum, right: Datum) -> Result<Datum> {
    match op {
        BinaryOp::Eq => compare_equality(left, right, false),
        BinaryOp::Ne => compare_equality(left, right, true),
        BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => compare_order(op, left, right),
        BinaryOp::And => truth_binary(left, right, Truth::and),
        BinaryOp::Or => truth_binary(left, right, Truth::or),
        BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Rem => {
            arithmetic(op, left, right)
        }
        BinaryOp::IsDistinctFrom => distinct_equality(left, right, true),
        BinaryOp::IsNotDistinctFrom => distinct_equality(left, right, false),
        BinaryOp::Coalesce => Ok(coalesce(left, right)),
    }
}

pub(crate) fn eval_membership(
    input: Datum,
    candidates: impl IntoIterator<Item = Datum>,
    negate: bool,
) -> Result<Datum> {
    let mut saw_unknown = false;
    for candidate in candidates {
        let comparison = compare_equality(input.clone(), candidate, false)?;
        match as_truth(comparison)? {
            Truth::True => return Ok(truth(!negate)),
            Truth::False => {}
            Truth::Unknown => saw_unknown = true,
        }
    }

    if saw_unknown {
        Ok(unknown())
    } else {
        Ok(truth(negate))
    }
}

pub(crate) fn eval_conditional(
    condition: Datum,
    when_true: Datum,
    when_false: Datum,
) -> Result<Datum> {
    match as_truth(condition)? {
        Truth::True => Ok(when_true),
        Truth::False | Truth::Unknown => Ok(when_false),
    }
}

pub(crate) fn eval_builtin_function(
    function: &FunctionDef,
    arguments: Vec<Datum>,
    _output: &TypeDef,
) -> Result<Datum> {
    match function.key().as_str() {
        "dol/text/to-lowercase" => text_unary(arguments, str::to_lowercase),
        "dol/text/to-uppercase" => text_unary(arguments, str::to_uppercase),
        "dol/text/trim" => text_unary(arguments, |value| value.trim().to_owned()),
        "dol/text/trim-start" => text_unary(arguments, |value| value.trim_start().to_owned()),
        "dol/text/trim-end" => text_unary(arguments, |value| value.trim_end().to_owned()),
        "dol/text/contains" | "dol/text/starts-with" | "dol/text/ends-with" => {
            text_predicate(function.key().as_str(), arguments)
        }
        "dol/text/byte-length" => unary_value(arguments, |value| match value {
            Value::String(value) => {
                let length = u64::try_from(value.len()).map_err(|_| {
                    Diagnostic::error(
                        "EXPR-EVAL-006",
                        "string byte length exceeds the portable u64 result domain",
                    )
                })?;
                Ok(Value::UInt(u128::from(length)))
            }
            _ => Err(function_value_error("byte-length", "string")),
        }),
        "dol/text/scalar-length" => unary_value(arguments, |value| match value {
            Value::String(value) => {
                let length = u64::try_from(value.chars().count()).map_err(|_| {
                    Diagnostic::error(
                        "EXPR-EVAL-006",
                        "string scalar length exceeds the portable u64 result domain",
                    )
                })?;
                Ok(Value::UInt(u128::from(length)))
            }
            _ => Err(function_value_error("scalar-length", "string")),
        }),
        "dol/date/year" => unary_value(arguments, |value| match value {
            Value::Date(value) => Ok(Value::Int(i128::from(value.year()))),
            _ => Err(function_value_error("date/year", "date")),
        }),
        "dol/date/month" => unary_value(arguments, |value| match value {
            Value::Date(value) => Ok(Value::UInt(u128::from(month_number(value.month())))),
            _ => Err(function_value_error("date/month", "date")),
        }),
        "dol/date/day" => unary_value(arguments, |value| match value {
            Value::Date(value) => Ok(Value::UInt(u128::from(value.day()))),
            _ => Err(function_value_error("date/day", "date")),
        }),
        "dol/date/ordinal" => unary_value(arguments, |value| match value {
            Value::Date(value) => Ok(Value::UInt(u128::from(value.ordinal()))),
            _ => Err(function_value_error("date/ordinal", "date")),
        }),
        "dol/date/weekday" => unary_value(arguments, |value| match value {
            Value::Date(value) => Ok(Value::UInt(u128::from(weekday_number(value.weekday())))),
            _ => Err(function_value_error("date/weekday", "date")),
        }),
        "dol/local-datetime/date" => unary_value(arguments, |value| match value {
            Value::LocalDateTime(value) => Ok(Value::Date(value.date())),
            _ => Err(function_value_error(
                "local-datetime/date",
                "local datetime",
            )),
        }),
        "dol/local-datetime/time" => unary_value(arguments, |value| match value {
            Value::LocalDateTime(value) => Ok(Value::Time(value.time())),
            _ => Err(function_value_error(
                "local-datetime/time",
                "local datetime",
            )),
        }),
        "dol/instant/unix-timestamp" => unary_value(arguments, |value| match value {
            Value::Instant(value) => Ok(Value::Int(i128::from(value.unix_timestamp()))),
            _ => Err(function_value_error("instant/unix-timestamp", "instant")),
        }),
        "dol/duration/whole-seconds" => unary_value(arguments, |value| match value {
            Value::Duration(value) => Ok(Value::Int(i128::from(value.whole_seconds()))),
            _ => Err(function_value_error("duration/whole-seconds", "duration")),
        }),
        key => Err(Diagnostic::error(
            "EXPR-EVAL-006",
            format!("no built-in local evaluator exists for semantic function `{key}`"),
        )),
    }
}

fn unary_value(
    arguments: Vec<Datum>,
    operation: impl FnOnce(Value) -> Result<Value>,
) -> Result<Datum> {
    let [input] = arguments.try_into().map_err(|_| {
        Diagnostic::error("EXPR-EVAL-006", "unary function received an invalid arity")
    })?;
    match input {
        Datum::Missing => Ok(Datum::Missing),
        Datum::Null => Ok(Datum::Null),
        Datum::Value(value) => operation(value).map(Datum::Value),
    }
}

fn text_unary(arguments: Vec<Datum>, operation: impl FnOnce(&str) -> String) -> Result<Datum> {
    unary_value(arguments, |value| match value {
        Value::String(value) => Ok(Value::String(operation(&value))),
        _ => Err(function_value_error("text", "string")),
    })
}

fn text_predicate(function: &str, arguments: Vec<Datum>) -> Result<Datum> {
    let [left, right] = arguments.try_into().map_err(|_| {
        Diagnostic::error(
            "EXPR-EVAL-006",
            "binary text function received an invalid arity",
        )
    })?;
    match (left, right) {
        (Datum::Missing | Datum::Null, _) | (_, Datum::Missing | Datum::Null) => Ok(unknown()),
        (Datum::Value(Value::String(left)), Datum::Value(Value::String(right))) => {
            let result = match function {
                "dol/text/contains" => left.contains(&right),
                "dol/text/starts-with" => left.starts_with(&right),
                "dol/text/ends-with" => left.ends_with(&right),
                _ => {
                    return Err(Diagnostic::error(
                        "EXPR-EVAL-006",
                        "non-text-predicate function reached text predicate evaluation",
                    ));
                }
            };
            Ok(truth(result))
        }
        _ => Err(function_value_error("text predicate", "string")),
    }
}

fn month_number(month: time::Month) -> u8 {
    match month {
        time::Month::January => 1,
        time::Month::February => 2,
        time::Month::March => 3,
        time::Month::April => 4,
        time::Month::May => 5,
        time::Month::June => 6,
        time::Month::July => 7,
        time::Month::August => 8,
        time::Month::September => 9,
        time::Month::October => 10,
        time::Month::November => 11,
        time::Month::December => 12,
    }
}

fn weekday_number(weekday: time::Weekday) -> u8 {
    match weekday {
        time::Weekday::Monday => 1,
        time::Weekday::Tuesday => 2,
        time::Weekday::Wednesday => 3,
        time::Weekday::Thursday => 4,
        time::Weekday::Friday => 5,
        time::Weekday::Saturday => 6,
        time::Weekday::Sunday => 7,
    }
}

fn function_value_error(function: &str, expected: &str) -> Diagnostic {
    Diagnostic::error(
        "EXPR-EVAL-006",
        format!("function `{function}` expected a {expected} runtime value"),
    )
}

pub(crate) fn validate_lossless_cast(source: &TypeDef, target: &TypeDef) -> Result<()> {
    if source.nullability() != target.nullability() {
        return Err(Diagnostic::error(
            "EXPR-CAST-001",
            "lossless cast must preserve nullability",
        ));
    }

    let source = source.clone().non_null();
    let target = target.clone().non_null();
    let (TypeShape::Scalar(source), TypeShape::Scalar(target)) = (source.shape(), target.shape())
    else {
        return Err(Diagnostic::error(
            "EXPR-CAST-002",
            "lossless cast requires scalar numeric types",
        ));
    };

    if lossless_scalar_cast(*source, *target) {
        Ok(())
    } else {
        Err(Diagnostic::error(
            "EXPR-CAST-003",
            "requested numeric cast is not lossless for every source value",
        ))
    }
}

fn lossless_scalar_cast(source: ScalarRepr, target: ScalarRepr) -> bool {
    match (source, target) {
        (ScalarRepr::Int { bits: source }, ScalarRepr::Int { bits: target }) => target >= source,
        (ScalarRepr::UInt { bits: source }, ScalarRepr::UInt { bits: target }) => target >= source,
        (ScalarRepr::UInt { bits: source }, ScalarRepr::Int { bits: target }) => target > source,
        (ScalarRepr::Float32, ScalarRepr::Float32 | ScalarRepr::Float64)
        | (ScalarRepr::Float64, ScalarRepr::Float64)
        | (ScalarRepr::Decimal, ScalarRepr::Decimal) => true,
        _ => false,
    }
}

fn cast_lossless(input: Datum, target: &TypeDef) -> Result<Datum> {
    match input {
        Datum::Missing => Ok(Datum::Missing),
        Datum::Null if target.nullability() == Nullability::Nullable => Ok(Datum::Null),
        Datum::Null => Err(Diagnostic::error(
            "EXPR-CAST-004",
            "lossless cast cannot produce null for a non-null target",
        )),
        Datum::Value(value) => cast_value_lossless(value, target).map(Datum::Value),
    }
}

fn cast_value_lossless(value: Value, target: &TypeDef) -> Result<Value> {
    let TypeShape::Scalar(target) = target.shape() else {
        return Err(Diagnostic::error(
            "EXPR-CAST-002",
            "lossless cast requires scalar numeric types",
        ));
    };

    match (value, target) {
        (Value::Int(value), ScalarRepr::Int { .. }) => Ok(Value::Int(value)),
        (Value::UInt(value), ScalarRepr::UInt { .. }) => Ok(Value::UInt(value)),
        (Value::UInt(value), ScalarRepr::Int { .. }) => {
            i128::try_from(value).map(Value::Int).map_err(|_| {
                Diagnostic::error("EXPR-CAST-005", "unsigned value does not fit signed target")
            })
        }
        (Value::Float32(value), ScalarRepr::Float32) => Ok(Value::Float32(value)),
        (Value::Float32(value), ScalarRepr::Float64) => Ok(Value::Float64(f64::from(value))),
        (Value::Float64(value), ScalarRepr::Float64) => Ok(Value::Float64(value)),
        (Value::Decimal(value), ScalarRepr::Decimal) => Ok(Value::Decimal(value)),
        _ => Err(Diagnostic::error(
            "EXPR-CAST-006",
            "runtime value is incompatible with the validated lossless cast",
        )),
    }
}

fn truth(value: bool) -> Datum {
    Datum::Value(Value::Truth(if value { Truth::True } else { Truth::False }))
}

fn unknown() -> Datum {
    Datum::Value(Value::Truth(Truth::Unknown))
}

fn truth_unary(input: Datum, operation: impl FnOnce(Truth) -> Truth) -> Result<Datum> {
    let value = as_truth(input)?;
    Ok(Datum::Value(Value::Truth(operation(value))))
}

fn truth_binary(
    left: Datum,
    right: Datum,
    operation: impl FnOnce(Truth, Truth) -> Truth,
) -> Result<Datum> {
    let left = as_truth(left)?;
    let right = as_truth(right)?;
    Ok(Datum::Value(Value::Truth(operation(left, right))))
}

fn as_truth(datum: Datum) -> Result<Truth> {
    match datum {
        Datum::Value(Value::Truth(value)) => Ok(value),
        _ => Err(Diagnostic::error(
            "EXPR-EVAL-001",
            "logical operation expected a truth value",
        )),
    }
}

fn compare_equality(left: Datum, right: Datum, negate: bool) -> Result<Datum> {
    match (left, right) {
        (Datum::Missing | Datum::Null, _) | (_, Datum::Missing | Datum::Null) => Ok(unknown()),
        (Datum::Value(left), Datum::Value(right)) => {
            let equal = left == right;
            Ok(truth(if negate { !equal } else { equal }))
        }
    }
}

fn distinct_equality(left: Datum, right: Datum, distinct: bool) -> Result<Datum> {
    let equal = match (left, right) {
        (Datum::Missing, Datum::Missing) | (Datum::Null, Datum::Null) => true,
        (Datum::Missing, _) | (_, Datum::Missing) | (Datum::Null, _) | (_, Datum::Null) => false,
        (Datum::Value(left), Datum::Value(right)) => left == right,
    };
    Ok(truth(if distinct { !equal } else { equal }))
}

fn compare_order(op: BinaryOp, left: Datum, right: Datum) -> Result<Datum> {
    let (Datum::Value(left), Datum::Value(right)) = (left, right) else {
        return Ok(unknown());
    };

    let Some(ordering) = value_partial_cmp(&left, &right) else {
        return Ok(truth(false));
    };
    let result = match op {
        BinaryOp::Lt => ordering == Ordering::Less,
        BinaryOp::Le => ordering != Ordering::Greater,
        BinaryOp::Gt => ordering == Ordering::Greater,
        BinaryOp::Ge => ordering != Ordering::Less,
        _ => false,
    };
    Ok(truth(result))
}

fn value_partial_cmp(left: &Value, right: &Value) -> Option<Ordering> {
    match (left, right) {
        (Value::Bool(left), Value::Bool(right)) => Some(left.cmp(right)),
        (Value::Truth(left), Value::Truth(right)) => Some(left.cmp(right)),
        (Value::Int(left), Value::Int(right)) => Some(left.cmp(right)),
        (Value::UInt(left), Value::UInt(right)) => Some(left.cmp(right)),
        (Value::Float32(left), Value::Float32(right)) => left.partial_cmp(right),
        (Value::Float64(left), Value::Float64(right)) => left.partial_cmp(right),
        (Value::Decimal(left), Value::Decimal(right)) => Some(left.cmp(right)),
        (Value::Char(left), Value::Char(right)) => Some(left.cmp(right)),
        (Value::String(left), Value::String(right)) => Some(left.cmp(right)),
        (Value::Bytes(left), Value::Bytes(right)) => Some(left.cmp(right)),
        (Value::Uuid(left), Value::Uuid(right)) => Some(left.cmp(right)),
        (Value::Date(left), Value::Date(right)) => Some(left.cmp(right)),
        (Value::Time(left), Value::Time(right)) => Some(left.cmp(right)),
        (Value::LocalDateTime(left), Value::LocalDateTime(right)) => Some(left.cmp(right)),
        (Value::Instant(left), Value::Instant(right)) => Some(left.cmp(right)),
        (Value::Duration(left), Value::Duration(right)) => Some(left.cmp(right)),
        _ => None,
    }
}

fn arithmetic(op: BinaryOp, left: Datum, right: Datum) -> Result<Datum> {
    match (left, right) {
        (Datum::Missing, _) | (_, Datum::Missing) => Ok(Datum::Missing),
        (Datum::Null, _) | (_, Datum::Null) => Ok(Datum::Null),
        (Datum::Value(left), Datum::Value(right)) => {
            arithmetic_values(op, left, right).map(Datum::Value)
        }
    }
}

fn arithmetic_values(op: BinaryOp, left: Value, right: Value) -> Result<Value> {
    match (left, right) {
        (Value::Int(left), Value::Int(right)) => checked_signed(op, left, right).map(Value::Int),
        (Value::UInt(left), Value::UInt(right)) => {
            checked_unsigned(op, left, right).map(Value::UInt)
        }
        (Value::Float32(left), Value::Float32(right)) => Ok(Value::Float32(match op {
            BinaryOp::Add => left + right,
            BinaryOp::Sub => left - right,
            BinaryOp::Mul => left * right,
            BinaryOp::Div => left / right,
            _ => return arithmetic_operator_error(),
        })),
        (Value::Float64(left), Value::Float64(right)) => Ok(Value::Float64(match op {
            BinaryOp::Add => left + right,
            BinaryOp::Sub => left - right,
            BinaryOp::Mul => left * right,
            BinaryOp::Div => left / right,
            _ => return arithmetic_operator_error(),
        })),
        (Value::Decimal(left), Value::Decimal(right)) => {
            let result = match op {
                BinaryOp::Add => left.checked_add(right),
                BinaryOp::Sub => left.checked_sub(right),
                BinaryOp::Mul => left.checked_mul(right),
                BinaryOp::Div => left.checked_div(right),
                _ => return arithmetic_operator_error(),
            };
            result.map(Value::Decimal).ok_or_else(|| {
                Diagnostic::error(
                    "EXPR-EVAL-003",
                    "decimal arithmetic overflow or invalid division",
                )
            })
        }
        _ => Err(Diagnostic::error(
            "EXPR-EVAL-002",
            "numeric operation received incompatible runtime values",
        )),
    }
}

fn checked_signed(op: BinaryOp, left: i128, right: i128) -> Result<i128> {
    let result = match op {
        BinaryOp::Add => left.checked_add(right),
        BinaryOp::Sub => left.checked_sub(right),
        BinaryOp::Mul => left.checked_mul(right),
        BinaryOp::Div => left.checked_div(right),
        BinaryOp::Rem => left.checked_rem(right),
        _ => return arithmetic_operator_error(),
    };
    result.ok_or_else(|| {
        Diagnostic::error(
            "EXPR-EVAL-003",
            "signed arithmetic overflow or invalid division/remainder",
        )
    })
}

fn checked_unsigned(op: BinaryOp, left: u128, right: u128) -> Result<u128> {
    let result = match op {
        BinaryOp::Add => left.checked_add(right),
        BinaryOp::Sub => left.checked_sub(right),
        BinaryOp::Mul => left.checked_mul(right),
        BinaryOp::Div => left.checked_div(right),
        BinaryOp::Rem => left.checked_rem(right),
        _ => return arithmetic_operator_error(),
    };
    result.ok_or_else(|| {
        Diagnostic::error(
            "EXPR-EVAL-003",
            "unsigned arithmetic overflow or invalid division/remainder",
        )
    })
}

fn numeric_abs(input: Datum) -> Result<Datum> {
    match input {
        Datum::Missing => Ok(Datum::Missing),
        Datum::Null => Ok(Datum::Null),
        Datum::Value(Value::Int(value)) => value
            .checked_abs()
            .map(Value::Int)
            .map(Datum::Value)
            .ok_or_else(|| {
                Diagnostic::error(
                    "EXPR-EVAL-003",
                    "signed arithmetic overflow during absolute value",
                )
            }),
        Datum::Value(Value::Float32(value)) => Ok(Datum::Value(Value::Float32(value.abs()))),
        Datum::Value(Value::Float64(value)) => Ok(Datum::Value(Value::Float64(value.abs()))),
        Datum::Value(Value::Decimal(value)) => Ok(Datum::Value(Value::Decimal(value.abs()))),
        Datum::Value(_) => Err(Diagnostic::error(
            "EXPR-EVAL-002",
            "numeric absolute value received an incompatible runtime value",
        )),
    }
}

fn numeric_negate(input: Datum) -> Result<Datum> {
    match input {
        Datum::Missing => Ok(Datum::Missing),
        Datum::Null => Ok(Datum::Null),
        Datum::Value(Value::Int(value)) => value
            .checked_neg()
            .map(Value::Int)
            .map(Datum::Value)
            .ok_or_else(|| {
                Diagnostic::error(
                    "EXPR-EVAL-003",
                    "signed arithmetic overflow during negation",
                )
            }),
        Datum::Value(Value::Float32(value)) => Ok(Datum::Value(Value::Float32(-value))),
        Datum::Value(Value::Float64(value)) => Ok(Datum::Value(Value::Float64(-value))),
        Datum::Value(Value::Decimal(value)) => Ok(Datum::Value(Value::Decimal(-value))),
        Datum::Value(_) => Err(Diagnostic::error(
            "EXPR-EVAL-002",
            "numeric negation received an incompatible runtime value",
        )),
    }
}

fn coalesce(left: Datum, right: Datum) -> Datum {
    match left {
        Datum::Missing | Datum::Null => right,
        value => value,
    }
}

fn arithmetic_operator_error<T>() -> Result<T> {
    Err(Diagnostic::error(
        "EXPR-EVAL-004",
        "unsupported arithmetic operator",
    ))
}
