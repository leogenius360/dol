//! Bounded frame parsing into private DTOs followed by validated core lowering.

use dol_core::model::{ModelDef, ModelSet};
use dol_core::types::{Presence, TypeDef};
use dol_core::value::Datum;

use crate::budget::DecodeLimits;
use crate::dto::{
    DatumDto, FieldDto, ModelDto, ParameterDto, RecordFieldDto, ReferenceDto, RelationDto,
    ScalarDto, ShapeDto, TypeDto, TypedDatumDto, ValueDto,
};
use crate::encode::{
    encode_model_def_with_limits, encode_model_set_with_limits, encode_type_def_with_limits,
    encode_typed_datum_with_limits,
};
use crate::envelope::{Envelope, HEADER_LEN, MAGIC, MAJOR_VERSION, MINOR_VERSION, PayloadKind};
use crate::error::{Result, WireError, WireErrorKind};
use crate::validate::{lower_model, lower_model_set, lower_type, lower_typed_datum};

/// Validated type, field-presence contract, and logical datum.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedDatum {
    ty: TypeDef,
    presence: Presence,
    datum: Datum,
}

impl TypedDatum {
    /// Creates and validates a typed datum.
    pub fn try_new(ty: TypeDef, presence: Presence, datum: Datum) -> Result<Self> {
        crate::validate::validate_type_for_encode(&ty, DecodeLimits::default())?;
        crate::validate::validate_datum_for_encode(&ty, presence, &datum)?;
        Ok(Self {
            ty,
            presence,
            datum,
        })
    }

    /// Semantic type definition.
    #[must_use]
    pub const fn ty(&self) -> &TypeDef {
        &self.ty
    }

    /// Field presence contract.
    #[must_use]
    pub const fn presence(&self) -> Presence {
        self.presence
    }

    /// Logical datum.
    #[must_use]
    pub const fn datum(&self) -> &Datum {
        &self.datum
    }

    /// Consumes this value into its three components.
    #[must_use]
    pub fn into_parts(self) -> (TypeDef, Presence, Datum) {
        (self.ty, self.presence, self.datum)
    }
}

/// Parses and validates only the fixed envelope and its declared payload length.
pub fn decode_envelope(input: &[u8], limits: DecodeLimits) -> Result<Envelope> {
    if input.len() > limits.max_total_bytes {
        return Err(limit_at(0, "frame exceeds max_total_bytes"));
    }
    if input.len() < HEADER_LEN {
        return Err(WireError::at(
            WireErrorKind::UnexpectedEof,
            input.len(),
            "frame is shorter than the fixed header",
        ));
    }
    if input[..4] != MAGIC {
        return Err(WireError::at(
            WireErrorKind::InvalidMagic,
            0,
            "invalid DOL wire magic",
        ));
    }
    let major_version = u16::from_be_bytes([input[4], input[5]]);
    let minor_version = u16::from_be_bytes([input[6], input[7]]);
    if major_version != MAJOR_VERSION || minor_version > MINOR_VERSION {
        return Err(WireError::at(
            WireErrorKind::UnsupportedVersion,
            4,
            format!("unsupported DOL wire version {major_version}.{minor_version}"),
        ));
    }
    let kind = PayloadKind::from_tag(input[8], 8)?;
    if input[9] != 0 || input[10] != 0 || input[11] != 0 {
        return Err(WireError::at(
            WireErrorKind::InvalidHeader,
            9,
            "flags and reserved header bytes must be zero",
        ));
    }
    let length = u64::from_be_bytes(
        input[12..20]
            .try_into()
            .expect("fixed header slice has eight bytes"),
    );
    let length = usize::try_from(length).map_err(|_| {
        WireError::at(
            WireErrorKind::LengthMismatch,
            12,
            "payload length exceeds this platform's address space",
        )
    })?;
    let expected = HEADER_LEN.checked_add(length).ok_or_else(|| {
        WireError::at(
            WireErrorKind::LengthMismatch,
            12,
            "declared frame length overflow",
        )
    })?;
    if expected != input.len() {
        return Err(WireError::at(
            WireErrorKind::LengthMismatch,
            12,
            format!(
                "declared payload is {length} bytes but frame contains {}",
                input.len() - HEADER_LEN
            ),
        ));
    }
    Ok(Envelope::decoded(
        major_version,
        minor_version,
        kind,
        input[HEADER_LEN..].to_vec(),
    ))
}

/// Decodes, validates, and canonicality-checks a type definition frame.
pub fn decode_type_def(input: &[u8], limits: DecodeLimits) -> Result<TypeDef> {
    let envelope = expected_envelope(input, limits, PayloadKind::TypeDef)?;
    let mut decoder = Decoder::new(envelope.payload(), limits);
    let dto = decoder.ty(0)?;
    decoder.finish()?;
    let value = lower_type(dto, limits)?;
    require_canonical(input, &encode_type_def_with_limits(&value, limits)?)?;
    Ok(value)
}

/// Decodes, validates, and canonicality-checks a typed datum frame.
pub fn decode_typed_datum(input: &[u8], limits: DecodeLimits) -> Result<TypedDatum> {
    let envelope = expected_envelope(input, limits, PayloadKind::TypedDatum)?;
    let mut decoder = Decoder::new(envelope.payload(), limits);
    let dto = TypedDatumDto {
        ty: decoder.ty(0)?,
        optional: decoder.bool()?,
        datum: decoder.datum(0)?,
    };
    decoder.finish()?;
    let (ty, presence, datum) = lower_typed_datum(dto, limits)?;
    require_canonical(
        input,
        &encode_typed_datum_with_limits(&ty, presence, &datum, limits)?,
    )?;
    Ok(TypedDatum {
        ty,
        presence,
        datum,
    })
}

/// Decodes, validates, and canonicality-checks one model definition frame.
pub fn decode_model_def(input: &[u8], limits: DecodeLimits) -> Result<ModelDef> {
    let envelope = expected_envelope(input, limits, PayloadKind::ModelDef)?;
    let mut decoder = Decoder::new(envelope.payload(), limits);
    let dto = decoder.model(0)?;
    decoder.finish()?;
    let value = lower_model(dto, limits)?;
    require_canonical(input, &encode_model_def_with_limits(&value, limits)?)?;
    Ok(value)
}

/// Decodes, cross-validates, and canonicality-checks a model-set frame.
pub fn decode_model_set(input: &[u8], limits: DecodeLimits) -> Result<ModelSet> {
    let envelope = expected_envelope(input, limits, PayloadKind::ModelSet)?;
    let mut decoder = Decoder::new(envelope.payload(), limits);
    let count = decoder.count(1)?;
    decoder.ensure_node_capacity(count)?;
    let mut models = Vec::with_capacity(count);
    for _ in 0..count {
        models.push(decoder.model(0)?);
    }
    decoder.finish()?;
    let value = lower_model_set(models, limits)?;
    require_canonical(input, &encode_model_set_with_limits(&value, limits)?)?;
    Ok(value)
}

fn expected_envelope(
    input: &[u8],
    limits: DecodeLimits,
    expected: PayloadKind,
) -> Result<Envelope> {
    let envelope = decode_envelope(input, limits)?;
    if envelope.kind() != expected {
        return Err(WireError::at(
            WireErrorKind::UnsupportedPayload,
            8,
            format!(
                "expected {:?} payload, found {:?}",
                expected,
                envelope.kind()
            ),
        ));
    }
    Ok(envelope)
}

fn require_canonical(input: &[u8], canonical: &[u8]) -> Result<()> {
    if input == canonical {
        Ok(())
    } else {
        Err(WireError::new(
            WireErrorKind::NonCanonical,
            "wire value is valid but is not in canonical encoding order or form",
        ))
    }
}

struct Decoder<'a> {
    input: &'a [u8],
    offset: usize,
    limits: DecodeLimits,
    nodes: usize,
}

impl<'a> Decoder<'a> {
    const fn new(input: &'a [u8], limits: DecodeLimits) -> Self {
        Self {
            input,
            offset: 0,
            limits,
            nodes: 0,
        }
    }

    const fn remaining(&self) -> usize {
        self.input.len() - self.offset
    }

    fn finish(&self) -> Result<()> {
        if self.offset == self.input.len() {
            Ok(())
        } else {
            Err(WireError::at(
                WireErrorKind::InvalidValue,
                self.offset,
                "payload contains trailing bytes",
            ))
        }
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8]> {
        let end = self.offset.checked_add(length).ok_or_else(|| {
            WireError::at(
                WireErrorKind::UnexpectedEof,
                self.offset,
                "field length overflow",
            )
        })?;
        if end > self.input.len() {
            return Err(WireError::at(
                WireErrorKind::UnexpectedEof,
                self.offset,
                format!(
                    "field needs {length} bytes but only {} remain",
                    self.remaining()
                ),
            ));
        }
        let bytes = &self.input[self.offset..end];
        self.offset = end;
        Ok(bytes)
    }

    fn fixed<const N: usize>(&mut self) -> Result<[u8; N]> {
        Ok(self
            .take(N)?
            .try_into()
            .expect("slice length was checked before conversion"))
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.fixed::<1>()?[0])
    }

    fn bool(&mut self) -> Result<bool> {
        let offset = self.offset;
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            tag => Err(invalid_tag(offset, "boolean", tag)),
        }
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.fixed()?))
    }

    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_be_bytes(self.fixed()?))
    }

    fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_be_bytes(self.fixed()?))
    }

    fn u128(&mut self) -> Result<u128> {
        Ok(u128::from_be_bytes(self.fixed()?))
    }

    fn i128(&mut self) -> Result<i128> {
        Ok(i128::from_be_bytes(self.fixed()?))
    }

    fn count(&mut self, minimum_item_bytes: usize) -> Result<usize> {
        let offset = self.offset;
        let count = usize::try_from(self.u32()?).expect("u32 always fits supported usize targets");
        if count > self.limits.max_list_items {
            return Err(limit_at(offset, "item count exceeds max_list_items"));
        }
        let minimum = count
            .checked_mul(minimum_item_bytes)
            .ok_or_else(|| limit_at(offset, "minimum collection representation length overflow"))?;
        if minimum > self.remaining() {
            return Err(WireError::at(
                WireErrorKind::UnexpectedEof,
                offset,
                "declared item count cannot fit in the remaining payload",
            ));
        }
        Ok(count)
    }

    fn string(&mut self) -> Result<String> {
        let offset = self.offset;
        let length = usize::try_from(self.u32()?).expect("u32 always fits supported usize targets");
        if length > self.limits.max_string_bytes {
            return Err(limit_at(offset, "string exceeds max_string_bytes"));
        }
        let bytes = self.take(length)?;
        let text = core::str::from_utf8(bytes).map_err(|_| {
            WireError::at(
                WireErrorKind::InvalidUtf8,
                offset + 4,
                "invalid UTF-8 string",
            )
        })?;
        Ok(text.to_owned())
    }

    fn literal(&mut self) -> Result<Vec<u8>> {
        let offset = self.offset;
        let length = usize::try_from(self.u32()?).expect("u32 always fits supported usize targets");
        if length > self.limits.max_literal_bytes {
            return Err(limit_at(offset, "literal exceeds max_literal_bytes"));
        }
        Ok(self.take(length)?.to_vec())
    }

    fn node(&mut self, depth: usize) -> Result<()> {
        if depth > self.limits.max_depth {
            return Err(limit_at(self.offset, "structure exceeds max_depth"));
        }
        self.nodes = self
            .nodes
            .checked_add(1)
            .ok_or_else(|| limit_at(self.offset, "node count overflow"))?;
        if self.nodes > self.limits.max_nodes {
            return Err(limit_at(self.offset, "structure exceeds max_nodes"));
        }
        Ok(())
    }

    fn ensure_node_capacity(&self, additional: usize) -> Result<()> {
        let required = self
            .nodes
            .checked_add(additional)
            .ok_or_else(|| limit_at(self.offset, "node count overflow"))?;
        if required > self.limits.max_nodes {
            return Err(limit_at(self.offset, "structure exceeds max_nodes"));
        }
        Ok(())
    }

    fn ty(&mut self, depth: usize) -> Result<TypeDto> {
        self.node(depth)?;
        let key = self.string()?;
        let version = self.u32()?;
        let nullable = self.bool()?;
        let property_bits = self.u8()?;
        let parameter_count = self.count(6)?;
        self.ensure_node_capacity(parameter_count)?;
        let mut parameters = Vec::with_capacity(parameter_count);
        for _ in 0..parameter_count {
            self.node(depth.saturating_add(1))?;
            let name = self.string()?;
            let tag_offset = self.offset;
            let parameter = match self.u8()? {
                0 => ParameterDto::Bool(self.bool()?),
                1 => ParameterDto::Int(self.i64()?),
                2 => ParameterDto::UInt(self.u64()?),
                3 => ParameterDto::String(self.string()?),
                tag => return Err(invalid_tag(tag_offset, "type parameter", tag)),
            };
            parameters.push((name, parameter));
        }
        let shape = self.shape(depth)?;
        Ok(TypeDto {
            key,
            version,
            nullable,
            property_bits,
            parameters,
            shape,
        })
    }

    fn shape(&mut self, depth: usize) -> Result<ShapeDto> {
        let offset = self.offset;
        let child_depth = depth.saturating_add(1);
        match self.u8()? {
            0 => self.scalar().map(ShapeDto::Scalar),
            1 => self.ty(child_depth).map(Box::new).map(ShapeDto::List),
            2 => Ok(ShapeDto::Map(
                Box::new(self.ty(child_depth)?),
                Box::new(self.ty(child_depth)?),
            )),
            3 => {
                let count = self.count(1)?;
                self.ensure_node_capacity(count)?;
                let mut elements = Vec::with_capacity(count);
                for _ in 0..count {
                    elements.push(self.ty(child_depth)?);
                }
                Ok(ShapeDto::Tuple(elements))
            }
            4 => {
                let count = self.count(6)?;
                self.ensure_node_capacity(count)?;
                let mut fields = Vec::with_capacity(count);
                for _ in 0..count {
                    self.node(child_depth)?;
                    fields.push(RecordFieldDto {
                        name: self.string()?,
                        optional: self.bool()?,
                        ty: self.ty(child_depth)?,
                    });
                }
                Ok(ShapeDto::Record(fields))
            }
            tag => Err(invalid_tag(offset, "type shape", tag)),
        }
    }

    fn scalar(&mut self) -> Result<ScalarDto> {
        let offset = self.offset;
        match self.u8()? {
            0 => Ok(ScalarDto::Bool),
            1 => Ok(ScalarDto::Truth),
            2 => self.u8().map(ScalarDto::Int),
            3 => self.u8().map(ScalarDto::UInt),
            4 => Ok(ScalarDto::Float32),
            5 => Ok(ScalarDto::Float64),
            6 => Ok(ScalarDto::Decimal),
            7 => Ok(ScalarDto::Char),
            8 => Ok(ScalarDto::String),
            9 => Ok(ScalarDto::Bytes),
            10 => Ok(ScalarDto::Uuid),
            11 => Ok(ScalarDto::Date),
            12 => Ok(ScalarDto::Time),
            13 => Ok(ScalarDto::LocalDateTime),
            14 => Ok(ScalarDto::Instant),
            15 => Ok(ScalarDto::Duration),
            tag => Err(invalid_tag(offset, "scalar representation", tag)),
        }
    }

    fn datum(&mut self, depth: usize) -> Result<DatumDto> {
        self.node(depth)?;
        let offset = self.offset;
        match self.u8()? {
            0 => Ok(DatumDto::Missing),
            1 => Ok(DatumDto::Null),
            2 => self.value(depth).map(DatumDto::Value),
            tag => Err(invalid_tag(offset, "datum state", tag)),
        }
    }

    fn value(&mut self, depth: usize) -> Result<ValueDto> {
        let offset = self.offset;
        let child_depth = depth.saturating_add(1);
        match self.u8()? {
            0 => self.bool().map(ValueDto::Bool),
            1 => self.u8().map(ValueDto::Truth),
            2 => self.i128().map(ValueDto::Int),
            3 => self.u128().map(ValueDto::UInt),
            4 => self.u32().map(ValueDto::Float32),
            5 => self.u64().map(ValueDto::Float64),
            6 => Ok(ValueDto::Decimal {
                mantissa: self.i128()?,
                scale: self.u32()?,
            }),
            7 => self.u32().map(ValueDto::Char),
            8 => self.string().map(ValueDto::String),
            9 => self.literal().map(ValueDto::Bytes),
            10 => self.fixed().map(ValueDto::Uuid),
            15 => self.i64().map(ValueDto::Date),
            16 => Ok(ValueDto::Time {
                hour: self.u8()?,
                minute: self.u8()?,
                second: self.u8()?,
                nanosecond: self.u32()?,
            }),
            17 => Ok(ValueDto::LocalDateTime {
                julian_day: self.i64()?,
                hour: self.u8()?,
                minute: self.u8()?,
                second: self.u8()?,
                nanosecond: self.u32()?,
            }),
            18 => self.i128().map(ValueDto::Instant),
            19 => self.i128().map(ValueDto::Duration),
            11 => self.datum_list(child_depth).map(ValueDto::List),
            12 => self.map(child_depth).map(ValueDto::Map),
            13 => self.datum_list(child_depth).map(ValueDto::Tuple),
            14 => {
                let count = self.count(5)?;
                self.ensure_node_capacity(count)?;
                let mut fields = Vec::with_capacity(count);
                for _ in 0..count {
                    fields.push((self.string()?, self.datum(child_depth)?));
                }
                Ok(ValueDto::Record(fields))
            }
            tag => Err(invalid_tag(offset, "logical value", tag)),
        }
    }

    fn datum_list(&mut self, depth: usize) -> Result<Vec<DatumDto>> {
        let count = self.count(1)?;
        self.ensure_node_capacity(count)?;
        let mut values = Vec::with_capacity(count);
        for _ in 0..count {
            values.push(self.datum(depth)?);
        }
        Ok(values)
    }

    fn map(&mut self, depth: usize) -> Result<Vec<(DatumDto, DatumDto)>> {
        let count = self.count(2)?;
        let nodes = count
            .checked_mul(2)
            .ok_or_else(|| limit_at(self.offset, "map node count overflow"))?;
        self.ensure_node_capacity(nodes)?;
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            entries.push((self.datum(depth)?, self.datum(depth)?));
        }
        Ok(entries)
    }

    fn model(&mut self, depth: usize) -> Result<ModelDto> {
        self.node(depth)?;
        let key = self.string()?;
        let name = self.string()?;
        let field_count = self.count(11)?;
        self.ensure_node_capacity(field_count)?;
        let mut fields = Vec::with_capacity(field_count);
        for _ in 0..field_count {
            self.node(depth.saturating_add(1))?;
            fields.push(FieldDto {
                key: self.string()?,
                name: self.string()?,
                optional: self.bool()?,
                ty: self.ty(depth.saturating_add(1))?,
            });
        }
        let identity_offset = self.offset;
        let identity = match self.u8()? {
            0 => None,
            1 => Some(self.string_keys(depth)?),
            tag => return Err(invalid_tag(identity_offset, "model identity option", tag)),
        };
        let unique_count = self.count(4)?;
        self.ensure_node_capacity(unique_count)?;
        let mut unique = Vec::with_capacity(unique_count);
        for _ in 0..unique_count {
            self.node(depth.saturating_add(1))?;
            unique.push(self.string_keys(depth)?);
        }
        let relation_count = self.count(16)?;
        self.ensure_node_capacity(relation_count)?;
        let mut relations = Vec::with_capacity(relation_count);
        for _ in 0..relation_count {
            self.node(depth.saturating_add(1))?;
            let key = self.string()?;
            let name = self.string()?;
            let target_model = self.string()?;
            let cardinality = self.u8()?;
            let pair_count = self.count(8)?;
            self.ensure_node_capacity(pair_count)?;
            let mut pairs = Vec::with_capacity(pair_count);
            for _ in 0..pair_count {
                self.node(depth.saturating_add(2))?;
                pairs.push((self.string()?, self.string()?));
            }
            relations.push(RelationDto {
                key,
                name,
                target_model,
                cardinality,
                fields: pairs,
            });
        }
        let reference_count = self.count(12)?;
        self.ensure_node_capacity(reference_count)?;
        let mut references = Vec::with_capacity(reference_count);
        for _ in 0..reference_count {
            self.node(depth.saturating_add(1))?;
            references.push(ReferenceDto {
                source_fields: self.string_keys(depth)?,
                target_model: self.string()?,
                target_fields: self.string_keys(depth)?,
            });
        }
        Ok(ModelDto {
            key,
            name,
            fields,
            identity,
            unique,
            relations,
            references,
        })
    }

    fn string_keys(&mut self, depth: usize) -> Result<Vec<String>> {
        let count = self.count(4)?;
        self.ensure_node_capacity(count)?;
        let mut keys = Vec::with_capacity(count);
        for _ in 0..count {
            self.node(depth.saturating_add(2))?;
            keys.push(self.string()?);
        }
        Ok(keys)
    }
}

fn invalid_tag(offset: usize, kind: &'static str, tag: u8) -> WireError {
    WireError::at(
        WireErrorKind::InvalidTag,
        offset,
        format!("invalid {kind} tag {tag}"),
    )
}

fn limit_at(offset: usize, message: &'static str) -> WireError {
    WireError::at(WireErrorKind::LimitExceeded, offset, message)
}
