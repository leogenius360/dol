//! DTO validation and lowering into DOL core values.

use std::collections::BTreeMap;

use dol_core::limits::DefinitionLimits;
use dol_core::model::{Cardinality, ModelBuilder, ModelDef, ModelSet, RelationDef};
use dol_core::semantics::Truth;
use dol_core::types::{
    Presence, RecordField, ScalarRepr, TypeDef, TypeParameter, TypeProperties, TypeShape,
    validate_type,
};
use dol_core::value::{Datum, Value, validate_datum};

use crate::DecodeLimits;
use crate::dto::{
    DatumDto, ModelDto, ParameterDto, ScalarDto, ShapeDto, TypeDto, TypedDatumDto, ValueDto,
};
use crate::error::{Result, WireError, WireErrorKind};

pub(crate) fn definition_limits(limits: DecodeLimits) -> DefinitionLimits {
    DefinitionLimits {
        max_fields: limits.max_list_items,
        max_relations: limits.max_list_items,
        max_constraints: limits.max_list_items,
        max_type_depth: limits.max_depth,
        max_type_nodes: limits.max_nodes,
        max_name_bytes: limits.max_string_bytes,
    }
}

pub(crate) fn lower_type(dto: TypeDto, limits: DecodeLimits) -> Result<TypeDef> {
    let ty = lower_type_unchecked(dto)?;
    validate_type(&ty, definition_limits(limits)).map_err(core_error)?;
    Ok(ty)
}

fn lower_type_unchecked(dto: TypeDto) -> Result<TypeDef> {
    let shape = match dto.shape {
        ShapeDto::Scalar(repr) => TypeShape::Scalar(lower_scalar(repr)),
        ShapeDto::List(element) => TypeShape::List(Box::new(lower_type_unchecked(*element)?)),
        ShapeDto::Map(key, value) => TypeShape::Map {
            key: Box::new(lower_type_unchecked(*key)?),
            value: Box::new(lower_type_unchecked(*value)?),
        },
        ShapeDto::Tuple(elements) => TypeShape::Tuple(
            elements
                .into_iter()
                .map(lower_type_unchecked)
                .collect::<Result<_>>()?,
        ),
        ShapeDto::Record(fields) => TypeShape::Record(
            fields
                .into_iter()
                .map(|field| {
                    let ty = lower_type_unchecked(field.ty)?;
                    Ok(if field.optional {
                        RecordField::optional(field.name, ty)
                    } else {
                        RecordField::required(field.name, ty)
                    })
                })
                .collect::<Result<_>>()?,
        ),
    };
    let properties = properties_from_bits(dto.property_bits);
    let mut ty = TypeDef::shaped(dto.key, dto.version, shape).with_properties(properties);
    if dto.nullable {
        ty = ty.nullable();
    }
    for (name, parameter) in dto.parameters {
        ty = ty.parameter(name, lower_parameter(parameter));
    }
    Ok(ty)
}

const fn lower_scalar(dto: ScalarDto) -> ScalarRepr {
    match dto {
        ScalarDto::Bool => ScalarRepr::Bool,
        ScalarDto::Truth => ScalarRepr::Truth,
        ScalarDto::Int(bits) => ScalarRepr::Int { bits },
        ScalarDto::UInt(bits) => ScalarRepr::UInt { bits },
        ScalarDto::Float32 => ScalarRepr::Float32,
        ScalarDto::Float64 => ScalarRepr::Float64,
        ScalarDto::Decimal => ScalarRepr::Decimal,
        ScalarDto::Char => ScalarRepr::Char,
        ScalarDto::String => ScalarRepr::String,
        ScalarDto::Bytes => ScalarRepr::Bytes,
        ScalarDto::Uuid => ScalarRepr::Uuid,
        ScalarDto::Date => ScalarRepr::Date,
        ScalarDto::Time => ScalarRepr::Time,
        ScalarDto::LocalDateTime => ScalarRepr::LocalDateTime,
        ScalarDto::Instant => ScalarRepr::Instant,
        ScalarDto::Duration => ScalarRepr::Duration,
    }
}

fn lower_parameter(dto: ParameterDto) -> TypeParameter {
    match dto {
        ParameterDto::Bool(value) => TypeParameter::Bool(value),
        ParameterDto::Int(value) => TypeParameter::Int(value),
        ParameterDto::UInt(value) => TypeParameter::UInt(value),
        ParameterDto::String(value) => TypeParameter::String(value),
    }
}

pub(crate) const fn properties_from_bits(bits: u8) -> TypeProperties {
    TypeProperties {
        equality: bits & (1 << 0) != 0,
        ordering: bits & (1 << 1) != 0,
        keyable: bits & (1 << 2) != 0,
        numeric: bits & (1 << 3) != 0,
        integral: bits & (1 << 4) != 0,
        exact_numeric: bits & (1 << 5) != 0,
        temporal: bits & (1 << 6) != 0,
        collection: bits & (1 << 7) != 0,
    }
}

pub(crate) fn properties_to_bits(properties: TypeProperties) -> u8 {
    u8::from(properties.equality)
        | (u8::from(properties.ordering) << 1)
        | (u8::from(properties.keyable) << 2)
        | (u8::from(properties.numeric) << 3)
        | (u8::from(properties.integral) << 4)
        | (u8::from(properties.exact_numeric) << 5)
        | (u8::from(properties.temporal) << 6)
        | (u8::from(properties.collection) << 7)
}

pub(crate) fn lower_typed_datum(
    dto: TypedDatumDto,
    limits: DecodeLimits,
) -> Result<(TypeDef, Presence, Datum)> {
    let ty = lower_type(dto.ty, limits)?;
    let presence = if dto.optional {
        Presence::Optional
    } else {
        Presence::Required
    };
    let datum = lower_datum(dto.datum)?;
    validate_datum(&ty, presence, &datum).map_err(core_error)?;
    Ok((ty, presence, datum))
}

fn lower_datum(dto: DatumDto) -> Result<Datum> {
    match dto {
        DatumDto::Missing => Ok(Datum::Missing),
        DatumDto::Null => Ok(Datum::Null),
        DatumDto::Value(value) => lower_value(value).map(Datum::Value),
    }
}

fn lower_value(dto: ValueDto) -> Result<Value> {
    match dto {
        ValueDto::Bool(value) => Ok(Value::Bool(value)),
        ValueDto::Truth(0) => Ok(Value::Truth(Truth::False)),
        ValueDto::Truth(1) => Ok(Value::Truth(Truth::True)),
        ValueDto::Truth(2) => Ok(Value::Truth(Truth::Unknown)),
        ValueDto::Truth(_) => Err(WireError::new(
            WireErrorKind::InvalidTag,
            "truth value has an invalid tag",
        )),
        ValueDto::Int(value) => Ok(Value::Int(value)),
        ValueDto::UInt(value) => Ok(Value::UInt(value)),
        ValueDto::Float32(bits) => Ok(Value::Float32(f32::from_bits(bits))),
        ValueDto::Float64(bits) => Ok(Value::Float64(f64::from_bits(bits))),
        ValueDto::Decimal { mantissa, scale } => lower_decimal(mantissa, scale),
        ValueDto::Char(value) => char::from_u32(value).map(Value::Char).ok_or_else(|| {
            WireError::new(WireErrorKind::InvalidValue, "invalid Unicode scalar value")
        }),
        ValueDto::String(value) => Ok(Value::String(value)),
        ValueDto::Bytes(value) => Ok(Value::Bytes(value)),
        ValueDto::Uuid(bytes) => lower_uuid(bytes),
        ValueDto::Date(julian_day) => {
            let julian_day = i32::try_from(julian_day).map_err(|_| invalid_temporal())?;
            time::Date::from_julian_day(julian_day)
                .map(Value::Date)
                .map_err(|_| invalid_temporal())
        }
        ValueDto::Time {
            hour,
            minute,
            second,
            nanosecond,
        } => time::Time::from_hms_nano(hour, minute, second, nanosecond)
            .map(Value::Time)
            .map_err(|_| invalid_temporal()),
        ValueDto::LocalDateTime {
            julian_day,
            hour,
            minute,
            second,
            nanosecond,
        } => {
            let julian_day = i32::try_from(julian_day).map_err(|_| invalid_temporal())?;
            let date = time::Date::from_julian_day(julian_day).map_err(|_| invalid_temporal())?;
            let time = time::Time::from_hms_nano(hour, minute, second, nanosecond)
                .map_err(|_| invalid_temporal())?;
            Ok(Value::LocalDateTime(time::PrimitiveDateTime::new(
                date, time,
            )))
        }
        ValueDto::Instant(nanoseconds) => {
            time::OffsetDateTime::from_unix_timestamp_nanos(nanoseconds)
                .map(Value::Instant)
                .map_err(|_| invalid_temporal())
        }
        ValueDto::Duration(nanoseconds) => {
            const NANOS_PER_SECOND: i128 = 1_000_000_000;
            let seconds =
                i64::try_from(nanoseconds / NANOS_PER_SECOND).map_err(|_| invalid_temporal())?;
            let subsecond =
                i32::try_from(nanoseconds % NANOS_PER_SECOND).map_err(|_| invalid_temporal())?;
            Ok(Value::Duration(time::Duration::new(seconds, subsecond)))
        }
        ValueDto::List(values) => lower_datums(values).map(Value::List),
        ValueDto::Map(entries) => entries
            .into_iter()
            .map(|(key, value)| Ok((lower_datum(key)?, lower_datum(value)?)))
            .collect::<Result<_>>()
            .map(Value::Map),
        ValueDto::Tuple(values) => lower_datums(values).map(Value::Tuple),
        ValueDto::Record(fields) => {
            let mut record = BTreeMap::new();
            for (name, datum) in fields {
                if record.insert(name, lower_datum(datum)?).is_some() {
                    return Err(WireError::new(
                        WireErrorKind::InvalidValue,
                        "record contains a duplicate field name",
                    ));
                }
            }
            Ok(Value::Record(record))
        }
    }
}

fn invalid_temporal() -> WireError {
    WireError::new(
        WireErrorKind::InvalidValue,
        "temporal value is outside the supported canonical range",
    )
}

fn lower_datums(values: Vec<DatumDto>) -> Result<Vec<Datum>> {
    values.into_iter().map(lower_datum).collect()
}

fn lower_decimal(mantissa: i128, scale: u32) -> Result<Value> {
    if scale > 28 {
        return Err(WireError::new(
            WireErrorKind::InvalidValue,
            "decimal scale exceeds 28",
        ));
    }
    let negative = mantissa.is_negative();
    let mut digits = mantissa.unsigned_abs().to_string();
    if scale > 0 {
        let scale = usize::try_from(scale).expect("decimal scale fits usize");
        if digits.len() <= scale {
            digits.insert_str(0, &"0".repeat(scale + 1 - digits.len()));
        }
        digits.insert(digits.len() - scale, '.');
    }
    if negative {
        digits.insert(0, '-');
    }
    let value = digits.parse().map_err(|_| {
        WireError::new(
            WireErrorKind::InvalidValue,
            "invalid decimal representation",
        )
    })?;
    Ok(Value::Decimal(value))
}

fn lower_uuid(bytes: [u8; 16]) -> Result<Value> {
    let mut text = String::with_capacity(36);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for (index, byte) in bytes.into_iter().enumerate() {
        if matches!(index, 4 | 6 | 8 | 10) {
            text.push('-');
        }
        text.push(char::from(HEX[usize::from(byte >> 4)]));
        text.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    let value = text
        .parse()
        .map_err(|_| WireError::new(WireErrorKind::InvalidValue, "invalid UUID representation"))?;
    Ok(Value::Uuid(value))
}

pub(crate) fn lower_model(dto: ModelDto, limits: DecodeLimits) -> Result<ModelDef> {
    let mut builder = ModelBuilder::with_key(dto.key, dto.name).limits(definition_limits(limits));
    for field in dto.fields {
        let ty = lower_type(field.ty, limits)?;
        let presence = if field.optional {
            Presence::Optional
        } else {
            Presence::Required
        };
        builder = builder.field_def(field.key, field.name, ty, presence);
    }
    if let Some(identity) = dto.identity {
        builder = builder.identity(identity);
    }
    for unique in dto.unique {
        builder = builder.unique(unique);
    }
    for relation in dto.relations {
        let cardinality = match relation.cardinality {
            0 => Cardinality::One,
            1 => Cardinality::OptionalOne,
            2 => Cardinality::Many,
            _ => {
                return Err(WireError::new(
                    WireErrorKind::InvalidTag,
                    "relation cardinality has an invalid tag",
                ));
            }
        };
        builder = builder.relation(RelationDef::new(
            relation.key,
            relation.name,
            relation.target_model,
            cardinality,
            relation.fields,
        ));
    }
    for reference in dto.references {
        builder = builder.reference(
            reference.source_fields,
            reference.target_model,
            reference.target_fields,
        );
    }
    builder.freeze().map_err(core_error)
}

pub(crate) fn lower_model_set(models: Vec<ModelDto>, limits: DecodeLimits) -> Result<ModelSet> {
    let models = models
        .into_iter()
        .map(|model| lower_model(model, limits))
        .collect::<Result<Vec<_>>>()?;
    ModelSet::new(models).map_err(core_error)
}

pub(crate) fn validate_type_for_encode(ty: &TypeDef, limits: DecodeLimits) -> Result<()> {
    validate_type(ty, definition_limits(limits)).map_err(core_error)
}

pub(crate) fn validate_datum_for_encode(
    ty: &TypeDef,
    presence: Presence,
    datum: &Datum,
) -> Result<()> {
    validate_datum(ty, presence, datum).map_err(core_error)
}

fn core_error(error: dol_core::diagnostic::Diagnostic) -> WireError {
    let kind = if error.message().contains("limit") || error.message().contains("exceeds ") {
        WireErrorKind::LimitExceeded
    } else {
        WireErrorKind::InvalidValue
    };
    WireError::new(kind, format!("{}: {}", error.code(), error.message()))
}
