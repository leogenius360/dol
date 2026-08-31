use std::collections::BTreeMap;

use crate::diagnostic::{Diagnostic, Result};
use crate::types::{Nullability, Presence, RecordField, ScalarRepr, TypeDef, TypeShape};

use super::{Datum, Value};

/// Validates one datum against a field's semantic type and presence contract.
pub fn validate_datum(ty: &TypeDef, presence: Presence, datum: &Datum) -> Result<()> {
    match datum {
        Datum::Missing if presence == Presence::Required => Err(Diagnostic::error(
            "RUNTIME-002",
            "required value is missing",
        )),
        Datum::Missing => Ok(()),
        Datum::Null if ty.nullability() == Nullability::NonNull => Err(Diagnostic::error(
            "RUNTIME-003",
            "non-null value received null",
        )),
        Datum::Null => Ok(()),
        Datum::Value(value) => validate_value(ty, value),
    }
}

/// Validates a concrete logical value against a semantic type.
pub fn validate_value(ty: &TypeDef, value: &Value) -> Result<()> {
    let compatible = match ty.shape() {
        TypeShape::Scalar(repr) => scalar_matches(*repr, value),
        TypeShape::List(element) => match value {
            Value::List(values) => {
                for item in values {
                    validate_nested(element, item)?;
                }
                true
            }
            _ => false,
        },
        TypeShape::Map {
            key,
            value: map_value,
        } => match value {
            Value::Map(entries) => {
                for (index, (entry_key, entry_value)) in entries.iter().enumerate() {
                    validate_nested(key, entry_key)?;
                    validate_nested(map_value, entry_value)?;
                    if entries[..index]
                        .iter()
                        .any(|(existing_key, _)| existing_key == entry_key)
                    {
                        return Err(Diagnostic::error(
                            "RUNTIME-007",
                            "map value contains a duplicate key",
                        ));
                    }
                }
                true
            }
            _ => false,
        },
        TypeShape::Tuple(elements) => match value {
            Value::Tuple(values) if values.len() == elements.len() => {
                for (element, datum) in elements.iter().zip(values) {
                    validate_nested(element, datum)?;
                }
                true
            }
            _ => false,
        },
        TypeShape::Record(fields) => match value {
            Value::Record(values) => validate_record_value(fields, values)?,
            _ => false,
        },
    };

    if compatible {
        Ok(())
    } else {
        Err(Diagnostic::error(
            "RUNTIME-004",
            format!("value does not conform to type `{}`", ty.key().as_str()),
        ))
    }
}

fn validate_nested(ty: &TypeDef, datum: &Datum) -> Result<()> {
    match datum {
        Datum::Missing => Err(Diagnostic::error(
            "RUNTIME-006",
            "missing is only valid for named record/model fields",
        )),
        Datum::Null if ty.nullability() == Nullability::Nullable => Ok(()),
        Datum::Null => Err(Diagnostic::error(
            "RUNTIME-003",
            "non-null nested value received null",
        )),
        Datum::Value(value) => validate_value(ty, value),
    }
}

fn scalar_matches(repr: ScalarRepr, value: &Value) -> bool {
    if let (ScalarRepr::Int { bits }, Value::Int(number)) = (repr, value) {
        return signed_fits(bits, *number);
    }
    if let (ScalarRepr::UInt { bits }, Value::UInt(number)) = (repr, value) {
        return unsigned_fits(bits, *number);
    }

    matches!(
        (repr, value),
        (ScalarRepr::Bool, Value::Bool(_))
            | (ScalarRepr::Truth, Value::Truth(_))
            | (ScalarRepr::Float32, Value::Float32(_))
            | (ScalarRepr::Float64, Value::Float64(_))
            | (ScalarRepr::Decimal, Value::Decimal(_))
            | (ScalarRepr::Char, Value::Char(_))
            | (ScalarRepr::String, Value::String(_))
            | (ScalarRepr::Bytes, Value::Bytes(_))
            | (ScalarRepr::Uuid, Value::Uuid(_))
            | (ScalarRepr::Date, Value::Date(_))
            | (ScalarRepr::Time, Value::Time(_))
            | (ScalarRepr::LocalDateTime, Value::LocalDateTime(_))
            | (ScalarRepr::Instant, Value::Instant(_))
            | (ScalarRepr::Duration, Value::Duration(_))
    )
}

fn signed_fits(bits: u8, value: i128) -> bool {
    match bits {
        8 => i8::try_from(value).is_ok(),
        16 => i16::try_from(value).is_ok(),
        32 => i32::try_from(value).is_ok(),
        64 => i64::try_from(value).is_ok(),
        128 => true,
        _ => false,
    }
}

fn unsigned_fits(bits: u8, value: u128) -> bool {
    match bits {
        8 => u8::try_from(value).is_ok(),
        16 => u16::try_from(value).is_ok(),
        32 => u32::try_from(value).is_ok(),
        64 => u64::try_from(value).is_ok(),
        128 => true,
        _ => false,
    }
}

fn validate_record_value(fields: &[RecordField], values: &BTreeMap<String, Datum>) -> Result<bool> {
    for key in values.keys() {
        if !fields.iter().any(|field| field.name() == key) {
            return Ok(false);
        }
    }
    for field in fields {
        match values.get(field.name()) {
            Some(datum) => validate_nested(field.ty(), datum)?,
            None if field.presence() == Presence::Optional => {}
            None => {
                return Err(Diagnostic::error(
                    "RUNTIME-005",
                    format!("record value missing field `{}`", field.name()),
                ));
            }
        }
    }
    Ok(true)
}
