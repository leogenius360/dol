//! Canonical PostgreSQL driver binds and state/value row decoding.

use std::collections::BTreeMap;
use std::sync::Arc;

use dol_core::diagnostic::{Diagnostic, Result};
use dol_core::model::ModelDef;
use dol_core::model::Presence;
use dol_core::plan::PlanOutput;
use dol_core::runtime::DynRow;
use dol_core::semantics::Truth;
use dol_core::types::{ScalarRepr, TypeDef, TypeShape};
use dol_core::value::{Datum, Value, validate_datum};
use dol_engine::ExecutionRow;
use postgres::Row;
use postgres::types::{FromSql, ToSql, Type};

use crate::compiler::scalar_repr;
use crate::sql::{CompiledQuery, SqlBind};

pub(crate) struct EncodedBinds {
    binds: Vec<EncodedBind>,
}

struct EncodedBind {
    value: Box<dyn ToSql + Sync>,
    ty: Type,
}

impl EncodedBinds {
    pub(crate) fn postgres_types(&self) -> Vec<Type> {
        self.binds.iter().map(|bind| bind.ty.clone()).collect()
    }

    pub(crate) fn values(&self) -> Vec<&(dyn ToSql + Sync)> {
        self.binds.iter().map(|bind| bind.value.as_ref()).collect()
    }

    pub(crate) fn bind_count(&self) -> usize {
        self.binds.len()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RowDecoder {
    model: Option<Arc<ModelDef>>,
}

impl RowDecoder {
    pub(crate) fn new(compiled: &CompiledQuery) -> Self {
        let model = match compiled.output() {
            PlanOutput::Model(model) => Some(Arc::new(model.as_ref().clone())),
            PlanOutput::Value(_) | PlanOutput::Product(_) | PlanOutput::Nullable(_) => None,
        };
        Self { model }
    }

    pub(crate) fn decode(&self, compiled: &CompiledQuery, row: &Row) -> Result<ExecutionRow> {
        decode_execution_row(compiled, row, self.model.as_ref())
    }
}

pub(crate) fn encode_binds(binds: &[SqlBind]) -> Result<EncodedBinds> {
    let binds = binds.iter().map(encode_bind).collect::<Result<Vec<_>>>()?;
    Ok(EncodedBinds { binds })
}

fn encode_bind(bind: &SqlBind) -> Result<EncodedBind> {
    match bind {
        SqlBind::State(value) => Ok(EncodedBind {
            value: Box::new(*value),
            ty: Type::INT2,
        }),
        SqlBind::Datum { ty, datum } => encode_datum(ty, datum),
    }
}

fn encode_datum(ty: &TypeDef, datum: &Datum) -> Result<EncodedBind> {
    let postgres_type = driver_bind_type(ty)?;
    let value = match datum {
        Datum::Missing | Datum::Null => encode_null(ty)?,
        Datum::Value(value) => encode_value(ty, value)?,
    };
    Ok(EncodedBind {
        value,
        ty: postgres_type,
    })
}

fn driver_bind_type(ty: &TypeDef) -> Result<Type> {
    let ty = match scalar_repr(ty)? {
        ScalarRepr::Bool => Type::BOOL,
        ScalarRepr::Truth => Type::INT2,
        ScalarRepr::Int { bits } if bits <= 16 => Type::INT2,
        ScalarRepr::Int { bits } if bits <= 32 => Type::INT4,
        ScalarRepr::Int { bits } if bits <= 64 => Type::INT8,
        ScalarRepr::Int { .. } | ScalarRepr::UInt { .. } | ScalarRepr::Decimal => Type::TEXT,
        ScalarRepr::Float32 => Type::FLOAT4,
        ScalarRepr::Float64 => Type::FLOAT8,
        ScalarRepr::Char | ScalarRepr::String => Type::TEXT,
        ScalarRepr::Bytes => Type::BYTEA,
        ScalarRepr::Uuid => Type::UUID,
        ScalarRepr::Date => Type::DATE,
        _ => return Err(unsupported_codec()),
    };
    Ok(ty)
}

fn encode_null(ty: &TypeDef) -> Result<Box<dyn ToSql + Sync>> {
    let value: Box<dyn ToSql + Sync> = match scalar_repr(ty)? {
        ScalarRepr::Bool => Box::new(None::<bool>),
        ScalarRepr::Truth => Box::new(None::<i16>),
        ScalarRepr::Int { bits } if bits <= 16 => Box::new(None::<i16>),
        ScalarRepr::Int { bits } if bits <= 32 => Box::new(None::<i32>),
        ScalarRepr::Int { bits } if bits <= 64 => Box::new(None::<i64>),
        ScalarRepr::Int { .. } | ScalarRepr::UInt { .. } | ScalarRepr::Decimal => {
            Box::new(None::<String>)
        }
        ScalarRepr::Float32 => Box::new(None::<f32>),
        ScalarRepr::Float64 => Box::new(None::<f64>),
        ScalarRepr::Char | ScalarRepr::String => Box::new(None::<String>),
        ScalarRepr::Bytes => Box::new(None::<Vec<u8>>),
        ScalarRepr::Uuid => Box::new(None::<uuid::Uuid>),
        ScalarRepr::Date => Box::new(None::<time::Date>),
        _ => return Err(unsupported_codec()),
    };
    Ok(value)
}

fn encode_value(ty: &TypeDef, value: &Value) -> Result<Box<dyn ToSql + Sync>> {
    let encoded: Box<dyn ToSql + Sync> = match (scalar_repr(ty)?, value) {
        (ScalarRepr::Bool, Value::Bool(value)) => Box::new(*value),
        (ScalarRepr::Truth, Value::Truth(value)) => Box::new(truth_tag(*value)),
        (ScalarRepr::Int { bits }, Value::Int(value)) if bits <= 16 => {
            Box::new(i16::try_from(*value).map_err(|_| invalid_bind())?)
        }
        (ScalarRepr::Int { bits }, Value::Int(value)) if bits <= 32 => {
            Box::new(i32::try_from(*value).map_err(|_| invalid_bind())?)
        }
        (ScalarRepr::Int { bits }, Value::Int(value)) if bits <= 64 => {
            Box::new(i64::try_from(*value).map_err(|_| invalid_bind())?)
        }
        (ScalarRepr::Int { .. }, Value::Int(value)) => Box::new(value.to_string()),
        (ScalarRepr::UInt { .. }, Value::UInt(value)) => Box::new(value.to_string()),
        (ScalarRepr::Decimal, Value::Decimal(value)) => Box::new(value.to_string()),
        (ScalarRepr::Float32, Value::Float32(value)) => Box::new(*value),
        (ScalarRepr::Float64, Value::Float64(value)) => Box::new(*value),
        (ScalarRepr::Char, Value::Char(value)) => Box::new(value.to_string()),
        (ScalarRepr::String, Value::String(value)) => Box::new(value.clone()),
        (ScalarRepr::Bytes, Value::Bytes(value)) => Box::new(value.clone()),
        (ScalarRepr::Uuid, Value::Uuid(value)) => Box::new(*value),
        (ScalarRepr::Date, Value::Date(value)) => Box::new(*value),
        _ => return Err(invalid_bind()),
    };
    Ok(encoded)
}

fn decode_execution_row(
    compiled: &CompiledQuery,
    row: &Row,
    model: Option<&Arc<ModelDef>>,
) -> Result<ExecutionRow> {
    if row.len() != compiled.columns().len().saturating_mul(2) {
        return Err(Diagnostic::error(
            "POSTGRES-DECODE-001",
            "PostgreSQL result column count differs from the compiled state/value layout",
        ));
    }

    let datums = compiled
        .columns()
        .iter()
        .enumerate()
        .map(|(index, column)| decode_datum(row, index * 2, column.type_def()))
        .collect::<Result<Vec<_>>>()?;

    decode_output(compiled.output(), datums, model)
}

fn decode_datum(row: &Row, state_index: usize, ty: &TypeDef) -> Result<Datum> {
    let state = get_optional::<i16>(row, state_index)?.ok_or_else(|| {
        Diagnostic::error(
            "POSTGRES-DECODE-002",
            "PostgreSQL result emitted NULL for a DOL state column",
        )
    })?;
    let datum = match state {
        0 => Datum::Missing,
        1 => Datum::Null,
        2 => Datum::Value(decode_value(row, state_index + 1, ty)?),
        _ => {
            return Err(Diagnostic::error(
                "POSTGRES-DECODE-003",
                "PostgreSQL result emitted an unknown DOL datum-state tag",
            ));
        }
    };
    Ok(datum)
}

fn decode_value(row: &Row, index: usize, ty: &TypeDef) -> Result<Value> {
    let value = match scalar_repr(ty)? {
        ScalarRepr::Bool => Value::Bool(required(row, index)?),
        ScalarRepr::Truth => Value::Truth(decode_truth(required(row, index)?)?),
        ScalarRepr::Int { bits } if bits <= 16 => {
            Value::Int(i128::from(required::<i16>(row, index)?))
        }
        ScalarRepr::Int { bits } if bits <= 32 => {
            Value::Int(i128::from(required::<i32>(row, index)?))
        }
        ScalarRepr::Int { bits } if bits <= 64 => {
            Value::Int(i128::from(required::<i64>(row, index)?))
        }
        ScalarRepr::Int { .. } => Value::Int(parse_text(row, index)?),
        ScalarRepr::UInt { .. } => Value::UInt(parse_text(row, index)?),
        ScalarRepr::Decimal => Value::Decimal(parse_text(row, index)?),
        ScalarRepr::Float32 => Value::Float32(required(row, index)?),
        ScalarRepr::Float64 => Value::Float64(required(row, index)?),
        ScalarRepr::Char => {
            let text = required::<String>(row, index)?;
            let mut chars = text.chars();
            let value = chars.next().ok_or_else(invalid_value)?;
            if chars.next().is_some() {
                return Err(invalid_value());
            }
            Value::Char(value)
        }
        ScalarRepr::String => Value::String(required(row, index)?),
        ScalarRepr::Bytes => Value::Bytes(required(row, index)?),
        ScalarRepr::Uuid => Value::Uuid(required(row, index)?),
        ScalarRepr::Date => Value::Date(required(row, index)?),
        _ => return Err(unsupported_codec()),
    };
    Ok(value)
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
                    "POSTGRES-DECODE-006",
                    format!(
                        "PostgreSQL projection violates DOL output semantics: {}",
                        error.message()
                    ),
                )
            })?;
            Ok(ExecutionRow::Value(datum))
        }
        PlanOutput::Product(_) | PlanOutput::Nullable(_) => Err(Diagnostic::error(
            "POSTGRES-DECODE-007",
            "PostgreSQL runtime received an output shape not supported by the current compiler",
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
            let fields = fields
                .iter()
                .zip(datums)
                .map(|(field, datum)| (field.name().to_owned(), datum))
                .collect::<BTreeMap<_, _>>();
            Ok(Datum::Value(Value::Record(fields)))
        }
        _ => Err(invalid_output()),
    }
}

fn get_optional<'a, T>(row: &'a Row, index: usize) -> Result<Option<T>>
where
    T: FromSql<'a>,
{
    row.try_get(index).map_err(driver_decode_error)
}

fn required<'a, T>(row: &'a Row, index: usize) -> Result<T>
where
    T: FromSql<'a>,
{
    get_optional(row, index)?.ok_or_else(invalid_value)
}

fn parse_text<T>(row: &Row, index: usize) -> Result<T>
where
    T: core::str::FromStr,
{
    required::<String>(row, index)?
        .parse::<T>()
        .map_err(|_| invalid_value())
}

const fn truth_tag(value: Truth) -> i16 {
    match value {
        Truth::False => 0,
        Truth::True => 1,
        Truth::Unknown => 2,
    }
}

fn decode_truth(value: i16) -> Result<Truth> {
    match value {
        0 => Ok(Truth::False),
        1 => Ok(Truth::True),
        2 => Ok(Truth::Unknown),
        _ => Err(invalid_value()),
    }
}

fn driver_decode_error(error: postgres::Error) -> Diagnostic {
    Diagnostic::error(
        "POSTGRES-DECODE-005",
        format!("PostgreSQL driver could not decode an expected result column: {error}"),
    )
}

fn invalid_bind() -> Diagnostic {
    Diagnostic::error(
        "POSTGRES-BIND-001",
        "canonical DOL bind value does not match its PostgreSQL scalar representation",
    )
}

fn unsupported_codec() -> Diagnostic {
    Diagnostic::error(
        "POSTGRES-BIND-002",
        "PostgreSQL runtime codec does not support this DOL scalar representation",
    )
}

fn invalid_value() -> Diagnostic {
    Diagnostic::error(
        "POSTGRES-DECODE-008",
        "PostgreSQL value cannot be represented exactly by the declared DOL scalar type",
    )
}

fn invalid_output() -> Diagnostic {
    Diagnostic::error(
        "POSTGRES-DECODE-009",
        "PostgreSQL result arity does not match the logical output type",
    )
}

#[cfg(test)]
mod tests {
    use dol_core::types::DataType;
    use postgres::types::Type;

    use super::*;

    #[test]
    fn encoded_binds_lock_exact_postgres_parameter_types() {
        let binds = [
            SqlBind::state(2),
            SqlBind::datum(u64::type_def(), Datum::Value(Value::UInt(42))),
            SqlBind::state(2),
            SqlBind::datum(i64::type_def(), Datum::Value(Value::Int(-7))),
            SqlBind::state(2),
            SqlBind::datum(
                String::type_def(),
                Datum::Value(Value::String("parameter".to_owned())),
            ),
        ];

        let encoded = encode_binds(&binds).unwrap();
        assert_eq!(encoded.bind_count(), binds.len());
        assert_eq!(
            encoded.postgres_types(),
            [
                Type::INT2,
                Type::TEXT,
                Type::INT2,
                Type::INT8,
                Type::INT2,
                Type::TEXT,
            ]
        );
        assert_eq!(encoded.values().len(), binds.len());
    }
}
