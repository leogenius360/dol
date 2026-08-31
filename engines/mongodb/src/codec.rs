//! Exact BSON scalar codec and state/value result decoding.

use std::collections::BTreeMap;
use std::sync::Arc;

use dol_core::diagnostic::{Diagnostic, Result};
use dol_core::model::{ModelDef, Presence};
use dol_core::plan::PlanOutput;
use dol_core::runtime::DynRow;
use dol_core::semantics::Truth;
use dol_core::types::{ScalarRepr, TypeDef, TypeShape};
use dol_core::value::{Datum, Value, validate_datum};
use dol_engine::ExecutionRow;
use mongodb::bson::{Binary, Bson, Decimal128, Document, spec::BinarySubtype};

use crate::compiled::CompiledAggregation;

/// Result decoder bound to one exact compiled output model.
#[derive(Debug, Clone)]
pub(crate) struct RowDecoder {
    model: Option<Arc<ModelDef>>,
}

impl RowDecoder {
    pub(crate) fn new(compiled: &CompiledAggregation) -> Self {
        let model = match compiled.output() {
            PlanOutput::Model(model) => Some(Arc::new(model.as_ref().clone())),
            PlanOutput::Value(_) | PlanOutput::Product(_) | PlanOutput::Nullable(_) => None,
        };
        Self { model }
    }

    pub(crate) fn decode(
        &self,
        compiled: &CompiledAggregation,
        document: &Document,
    ) -> Result<ExecutionRow> {
        let datums = compiled
            .fields()
            .iter()
            .map(|field| {
                let state = decode_state(document.get(field.state_field()))?;
                match state {
                    0 => Ok(Datum::Missing),
                    1 => Ok(Datum::Null),
                    2 => {
                        let value = document
                            .get(field.value_field())
                            .ok_or_else(invalid_value)?;
                        decode_value(field.type_def(), value).map(Datum::Value)
                    }
                    _ => Err(invalid_state()),
                }
            })
            .collect::<Result<Vec<_>>>()?;
        decode_output(compiled.output(), datums, self.model.as_ref())
    }
}

/// Rejects semantic representations for which this adapter has no lossless BSON contract.
pub(crate) fn ensure_supported_type(ty: &TypeDef) -> Result<()> {
    match ty.shape() {
        TypeShape::Scalar(
            ScalarRepr::Bool
            | ScalarRepr::Truth
            | ScalarRepr::Float64
            | ScalarRepr::Char
            | ScalarRepr::String
            | ScalarRepr::Bytes
            | ScalarRepr::Uuid,
        ) => Ok(()),
        TypeShape::Scalar(ScalarRepr::Int { bits }) if *bits <= 64 => Ok(()),
        // Unsigned integers through u64 use BSON Decimal128 so their full domain
        // is preserved rather than silently truncating into signed BSON integers.
        TypeShape::Scalar(ScalarRepr::UInt { bits }) if *bits <= 64 => Ok(()),
        _ => Err(Diagnostic::error(
            "MONGODB-CODEC-001",
            "semantic type has no lossless MongoDB BSON representation in the current adapter",
        )),
    }
}

pub(crate) fn encode_datum(ty: &TypeDef, datum: &Datum) -> Result<Bson> {
    ensure_supported_type(ty)?;
    validate_datum(ty, Presence::Optional, datum).map_err(|error| {
        Diagnostic::error(
            "MONGODB-CODEC-002",
            format!("invalid typed MongoDB bind: {}", error.message()),
        )
    })?;
    match datum {
        Datum::Missing | Datum::Null => Ok(Bson::Null),
        Datum::Value(value) => encode_value(ty, value),
    }
}

fn encode_value(ty: &TypeDef, value: &Value) -> Result<Bson> {
    let encoded = match (ty.shape(), value) {
        (TypeShape::Scalar(ScalarRepr::Bool), Value::Bool(value)) => Bson::Boolean(*value),
        (TypeShape::Scalar(ScalarRepr::Truth), Value::Truth(value)) => {
            Bson::Int32(truth_tag(*value))
        }
        (TypeShape::Scalar(ScalarRepr::Int { bits }), Value::Int(value)) if *bits <= 64 => {
            Bson::Int64(i64::try_from(*value).map_err(|_| invalid_value())?)
        }
        (TypeShape::Scalar(ScalarRepr::UInt { bits }), Value::UInt(value)) if *bits <= 64 => {
            Bson::Decimal128(
                value
                    .to_string()
                    .parse::<Decimal128>()
                    .map_err(|_| invalid_value())?,
            )
        }
        (TypeShape::Scalar(ScalarRepr::Float32), Value::Float32(value)) => {
            Bson::Double(f64::from(*value))
        }
        (TypeShape::Scalar(ScalarRepr::Float64), Value::Float64(value)) => Bson::Double(*value),
        (TypeShape::Scalar(ScalarRepr::Decimal), Value::Decimal(value)) => Bson::Decimal128(
            value
                .to_string()
                .parse::<Decimal128>()
                .map_err(|_| invalid_value())?,
        ),
        (TypeShape::Scalar(ScalarRepr::Char), Value::Char(value)) => {
            Bson::String(value.to_string())
        }
        (TypeShape::Scalar(ScalarRepr::String), Value::String(value)) => {
            Bson::String(value.clone())
        }
        (TypeShape::Scalar(ScalarRepr::Bytes), Value::Bytes(value)) => Bson::Binary(Binary {
            subtype: BinarySubtype::Generic,
            bytes: value.clone(),
        }),
        // A canonical textual UUID avoids dependence on server UUID representation
        // mode while remaining lossless and deterministic.
        (TypeShape::Scalar(ScalarRepr::Uuid), Value::Uuid(value)) => {
            Bson::String(value.hyphenated().to_string())
        }
        _ => return Err(invalid_value()),
    };
    Ok(encoded)
}

fn decode_value(ty: &TypeDef, value: &Bson) -> Result<Value> {
    ensure_supported_type(ty)?;
    let decoded = match (ty.shape(), value) {
        (TypeShape::Scalar(ScalarRepr::Bool), Bson::Boolean(value)) => Value::Bool(*value),
        (TypeShape::Scalar(ScalarRepr::Truth), Bson::Int32(value)) => {
            Value::Truth(decode_truth(*value)?)
        }
        (TypeShape::Scalar(ScalarRepr::Truth), Bson::Int64(value)) => Value::Truth(decode_truth(
            i32::try_from(*value).map_err(|_| invalid_value())?,
        )?),
        (TypeShape::Scalar(ScalarRepr::Int { bits }), Bson::Int32(value)) if *bits <= 64 => {
            Value::Int(i128::from(*value))
        }
        (TypeShape::Scalar(ScalarRepr::Int { bits }), Bson::Int64(value)) if *bits <= 64 => {
            Value::Int(i128::from(*value))
        }
        (TypeShape::Scalar(ScalarRepr::UInt { bits }), Bson::Decimal128(value)) if *bits <= 64 => {
            Value::UInt(value.to_string().parse().map_err(|_| invalid_value())?)
        }
        (TypeShape::Scalar(ScalarRepr::Float32), Bson::Double(value)) => {
            let narrowed = *value as f32;
            if !value.is_nan() && f64::from(narrowed) != *value {
                return Err(invalid_value());
            }
            Value::Float32(narrowed)
        }
        (TypeShape::Scalar(ScalarRepr::Float64), Bson::Double(value)) => Value::Float64(*value),
        (TypeShape::Scalar(ScalarRepr::Decimal), Bson::Decimal128(value)) => {
            Value::Decimal(value.to_string().parse().map_err(|_| invalid_value())?)
        }
        (TypeShape::Scalar(ScalarRepr::Char), Bson::String(value)) => {
            let mut chars = value.chars();
            let character = chars.next().ok_or_else(invalid_value)?;
            if chars.next().is_some() {
                return Err(invalid_value());
            }
            Value::Char(character)
        }
        (TypeShape::Scalar(ScalarRepr::String), Bson::String(value)) => {
            Value::String(value.clone())
        }
        (TypeShape::Scalar(ScalarRepr::Bytes), Bson::Binary(value))
            if value.subtype == BinarySubtype::Generic =>
        {
            Value::Bytes(value.bytes.clone())
        }
        (TypeShape::Scalar(ScalarRepr::Uuid), Bson::String(value)) => {
            Value::Uuid(value.parse().map_err(|_| invalid_value())?)
        }
        _ => return Err(invalid_value()),
    };
    Ok(decoded)
}

fn decode_output(
    output: &PlanOutput,
    datums: Vec<Datum>,
    model: Option<&Arc<ModelDef>>,
) -> Result<ExecutionRow> {
    match output {
        PlanOutput::Model(expected) => {
            if datums.len() != expected.fields().len() {
                return Err(invalid_output());
            }
            let model = model.ok_or_else(invalid_output)?;
            Ok(ExecutionRow::Model(DynRow::from_dense(
                Arc::clone(model),
                datums.into_boxed_slice(),
            )?))
        }
        PlanOutput::Value(ty) => {
            let datum = assemble_value_output(ty, datums)?;
            validate_datum(ty, Presence::Required, &datum).map_err(|error| {
                Diagnostic::error(
                    "MONGODB-DECODE-004",
                    format!(
                        "MongoDB projection violates DOL output semantics: {}",
                        error.message()
                    ),
                )
            })?;
            Ok(ExecutionRow::Value(datum))
        }
        PlanOutput::Product(_) | PlanOutput::Nullable(_) => Err(Diagnostic::error(
            "MONGODB-DECODE-005",
            "MongoDB compiler produced an unsupported product/nullable output",
        )),
    }
}

fn assemble_value_output(ty: &TypeDef, datums: Vec<Datum>) -> Result<Datum> {
    match ty.shape() {
        TypeShape::Scalar(_) if datums.len() == 1 => {
            datums.into_iter().next().ok_or_else(invalid_output)
        }
        TypeShape::Tuple(elements) if elements.len() == datums.len() => {
            Ok(Datum::Value(Value::Tuple(datums)))
        }
        TypeShape::Record(fields) if fields.len() == datums.len() => {
            let values = fields
                .iter()
                .zip(datums)
                .map(|(field, datum)| (field.name().to_owned(), datum))
                .collect::<BTreeMap<_, _>>();
            Ok(Datum::Value(Value::Record(values)))
        }
        _ => Err(invalid_output()),
    }
}

fn decode_state(value: Option<&Bson>) -> Result<i32> {
    match value {
        Some(Bson::Int32(value)) => Ok(*value),
        Some(Bson::Int64(value)) => i32::try_from(*value).map_err(|_| invalid_state()),
        _ => Err(invalid_state()),
    }
}

const fn truth_tag(value: Truth) -> i32 {
    match value {
        Truth::False => 0,
        Truth::True => 1,
        Truth::Unknown => 2,
    }
}

fn decode_truth(value: i32) -> Result<Truth> {
    match value {
        0 => Ok(Truth::False),
        1 => Ok(Truth::True),
        2 => Ok(Truth::Unknown),
        _ => Err(invalid_value()),
    }
}

fn invalid_state() -> Diagnostic {
    Diagnostic::error(
        "MONGODB-DECODE-001",
        "MongoDB result contains an invalid DOL datum-state tag",
    )
}

fn invalid_value() -> Diagnostic {
    Diagnostic::error(
        "MONGODB-DECODE-002",
        "BSON value does not match its exact DOL semantic representation",
    )
}

fn invalid_output() -> Diagnostic {
    Diagnostic::error(
        "MONGODB-DECODE-003",
        "MongoDB result layout differs from the compiled logical output",
    )
}

#[cfg(test)]
mod tests {
    use dol_core::semantics::Truth;
    use dol_core::types::{ScalarRepr, TypeDef};
    use dol_core::value::{Datum, Value};

    use super::{decode_value, encode_datum};

    #[test]
    fn scalar_codec_round_trips_truth_and_bytes() {
        let truth = TypeDef::scalar("test/truth", 1, ScalarRepr::Truth);
        let encoded = encode_datum(&truth, &Datum::Value(Value::Truth(Truth::Unknown))).unwrap();
        assert_eq!(
            decode_value(&truth, &encoded).unwrap(),
            Value::Truth(Truth::Unknown)
        );

        let bytes = TypeDef::scalar("test/bytes", 1, ScalarRepr::Bytes);
        let encoded =
            encode_datum(&bytes, &Datum::Value(Value::Bytes(vec![0, 1, 2, 255]))).unwrap();
        assert_eq!(
            decode_value(&bytes, &encoded).unwrap(),
            Value::Bytes(vec![0, 1, 2, 255])
        );
    }
}
