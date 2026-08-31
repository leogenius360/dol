//! Canonical binary encoding for validated DOL DTOs.

use dol_core::model::{Cardinality, ModelDef, ModelSet};
use dol_core::semantics::Truth;
use dol_core::types::{Nullability, Presence, ScalarRepr, TypeDef, TypeParameter, TypeShape};
use dol_core::value::{Datum, Value};

use crate::budget::DecodeLimits;
use crate::envelope::{Envelope, HEADER_LEN, MAGIC, MAJOR_VERSION, MINOR_VERSION, PayloadKind};
use crate::error::{Result, WireError, WireErrorKind};
use crate::validate::{properties_to_bits, validate_datum_for_encode, validate_type_for_encode};

/// Encodes an envelope using the fixed, versioned DOL frame header.
pub fn encode_envelope(envelope: &Envelope, limits: DecodeLimits) -> Result<Vec<u8>> {
    if envelope.major_version() != MAJOR_VERSION || envelope.minor_version() > MINOR_VERSION {
        return Err(WireError::new(
            WireErrorKind::UnsupportedVersion,
            "only DOL wire version 1.0 can be encoded",
        ));
    }
    let total = HEADER_LEN
        .checked_add(envelope.payload().len())
        .ok_or_else(|| limit_error("frame length overflow"))?;
    if total > limits.max_total_bytes {
        return Err(limit_error("frame exceeds max_total_bytes"));
    }
    let payload_len = u64::try_from(envelope.payload().len())
        .map_err(|_| limit_error("payload length exceeds the wire address space"))?;
    let mut output = Vec::with_capacity(total);
    output.extend_from_slice(&MAGIC);
    output.extend_from_slice(&envelope.major_version().to_be_bytes());
    output.extend_from_slice(&envelope.minor_version().to_be_bytes());
    output.push(envelope.kind().tag());
    output.push(0); // flags
    output.extend_from_slice(&0_u16.to_be_bytes()); // reserved
    output.extend_from_slice(&payload_len.to_be_bytes());
    output.extend_from_slice(envelope.payload());
    Ok(output)
}

/// Encodes a type definition with default limits.
pub fn encode_type_def(ty: &TypeDef) -> Result<Vec<u8>> {
    encode_type_def_with_limits(ty, DecodeLimits::default())
}

/// Encodes a type definition with explicit resource limits.
pub fn encode_type_def_with_limits(ty: &TypeDef, limits: DecodeLimits) -> Result<Vec<u8>> {
    validate_type_for_encode(ty, limits)?;
    let mut encoder = Encoder::new(limits)?;
    encoder.ty(ty, 0)?;
    finish(PayloadKind::TypeDef, encoder, limits)
}

/// Encodes a type, presence contract, and conforming datum with default limits.
pub fn encode_typed_datum(ty: &TypeDef, presence: Presence, datum: &Datum) -> Result<Vec<u8>> {
    encode_typed_datum_with_limits(ty, presence, datum, DecodeLimits::default())
}

/// Encodes a type, presence contract, and conforming datum with explicit limits.
pub fn encode_typed_datum_with_limits(
    ty: &TypeDef,
    presence: Presence,
    datum: &Datum,
    limits: DecodeLimits,
) -> Result<Vec<u8>> {
    validate_type_for_encode(ty, limits)?;
    validate_datum_for_encode(ty, presence, datum)?;
    let mut encoder = Encoder::new(limits)?;
    encoder.ty(ty, 0)?;
    encoder.bool(matches!(presence, Presence::Optional))?;
    encoder.datum(datum, 0)?;
    finish(PayloadKind::TypedDatum, encoder, limits)
}

/// Encodes one immutable model definition with default limits.
pub fn encode_model_def(model: &ModelDef) -> Result<Vec<u8>> {
    encode_model_def_with_limits(model, DecodeLimits::default())
}

/// Encodes one immutable model definition with explicit limits.
pub fn encode_model_def_with_limits(model: &ModelDef, limits: DecodeLimits) -> Result<Vec<u8>> {
    let mut encoder = Encoder::new(limits)?;
    encoder.model(model, 0)?;
    finish(PayloadKind::ModelDef, encoder, limits)
}

/// Encodes a validated model set with default limits.
pub fn encode_model_set(models: &ModelSet) -> Result<Vec<u8>> {
    encode_model_set_with_limits(models, DecodeLimits::default())
}

/// Encodes a validated model set with explicit limits.
pub fn encode_model_set_with_limits(models: &ModelSet, limits: DecodeLimits) -> Result<Vec<u8>> {
    let mut encoder = Encoder::new(limits)?;
    let count = models.iter().count();
    encoder.count(count)?;
    encoder.ensure_node_capacity(count)?;
    for model in models.iter() {
        encoder.model(model, 0)?;
    }
    finish(PayloadKind::ModelSet, encoder, limits)
}

fn finish(kind: PayloadKind, encoder: Encoder, limits: DecodeLimits) -> Result<Vec<u8>> {
    encode_envelope(&Envelope::new(kind, encoder.output), limits)
}

struct Encoder {
    limits: DecodeLimits,
    max_payload_bytes: usize,
    nodes: usize,
    output: Vec<u8>,
}

impl Encoder {
    fn new(limits: DecodeLimits) -> Result<Self> {
        let max_payload_bytes = limits
            .max_total_bytes
            .checked_sub(HEADER_LEN)
            .ok_or_else(|| limit_error("max_total_bytes is smaller than the frame header"))?;
        Ok(Self {
            limits,
            max_payload_bytes,
            nodes: 0,
            output: Vec::new(),
        })
    }

    fn write(&mut self, bytes: &[u8]) -> Result<()> {
        let next = self
            .output
            .len()
            .checked_add(bytes.len())
            .ok_or_else(|| limit_error("encoded payload length overflow"))?;
        if next > self.max_payload_bytes {
            return Err(limit_error("encoded frame exceeds max_total_bytes"));
        }
        self.output.extend_from_slice(bytes);
        Ok(())
    }

    fn u8(&mut self, value: u8) -> Result<()> {
        self.write(&[value])
    }

    fn bool(&mut self, value: bool) -> Result<()> {
        self.u8(u8::from(value))
    }

    fn u32(&mut self, value: u32) -> Result<()> {
        self.write(&value.to_be_bytes())
    }

    fn u64(&mut self, value: u64) -> Result<()> {
        self.write(&value.to_be_bytes())
    }

    fn i64(&mut self, value: i64) -> Result<()> {
        self.write(&value.to_be_bytes())
    }

    fn u128(&mut self, value: u128) -> Result<()> {
        self.write(&value.to_be_bytes())
    }

    fn i128(&mut self, value: i128) -> Result<()> {
        self.write(&value.to_be_bytes())
    }

    fn count(&mut self, value: usize) -> Result<()> {
        if value > self.limits.max_list_items {
            return Err(limit_error("item count exceeds max_list_items"));
        }
        let value = u32::try_from(value)
            .map_err(|_| limit_error("item count exceeds the wire address space"))?;
        self.u32(value)
    }

    fn string(&mut self, value: &str) -> Result<()> {
        if value.len() > self.limits.max_string_bytes {
            return Err(limit_error("string exceeds max_string_bytes"));
        }
        let length = u32::try_from(value.len())
            .map_err(|_| limit_error("string length exceeds the wire address space"))?;
        self.u32(length)?;
        self.write(value.as_bytes())
    }

    fn literal(&mut self, value: &[u8]) -> Result<()> {
        if value.len() > self.limits.max_literal_bytes {
            return Err(limit_error("literal exceeds max_literal_bytes"));
        }
        let length = u32::try_from(value.len())
            .map_err(|_| limit_error("literal length exceeds the wire address space"))?;
        self.u32(length)?;
        self.write(value)
    }

    fn node(&mut self, depth: usize) -> Result<()> {
        if depth > self.limits.max_depth {
            return Err(limit_error("structure exceeds max_depth"));
        }
        self.nodes = self
            .nodes
            .checked_add(1)
            .ok_or_else(|| limit_error("node count overflow"))?;
        if self.nodes > self.limits.max_nodes {
            return Err(limit_error("structure exceeds max_nodes"));
        }
        Ok(())
    }

    fn ensure_node_capacity(&self, additional: usize) -> Result<()> {
        let required = self
            .nodes
            .checked_add(additional)
            .ok_or_else(|| limit_error("node count overflow"))?;
        if required > self.limits.max_nodes {
            return Err(limit_error("structure exceeds max_nodes"));
        }
        Ok(())
    }

    fn ty(&mut self, ty: &TypeDef, depth: usize) -> Result<()> {
        self.node(depth)?;
        self.string(ty.key().as_str())?;
        self.u32(ty.version())?;
        self.bool(matches!(ty.nullability(), Nullability::Nullable))?;
        self.u8(properties_to_bits(ty.properties()))?;
        self.count(ty.parameters().len())?;
        self.ensure_node_capacity(ty.parameters().len())?;
        for (name, parameter) in ty.parameters() {
            self.node(depth.saturating_add(1))?;
            self.string(name)?;
            match parameter {
                TypeParameter::Bool(value) => {
                    self.u8(0)?;
                    self.bool(*value)?;
                }
                TypeParameter::Int(value) => {
                    self.u8(1)?;
                    self.i64(*value)?;
                }
                TypeParameter::UInt(value) => {
                    self.u8(2)?;
                    self.u64(*value)?;
                }
                TypeParameter::String(value) => {
                    self.u8(3)?;
                    self.string(value)?;
                }
                _ => return Err(unsupported("unknown type parameter variant")),
            }
        }
        self.shape(ty.shape(), depth)
    }

    fn shape(&mut self, shape: &TypeShape, depth: usize) -> Result<()> {
        let child_depth = depth.saturating_add(1);
        match shape {
            TypeShape::Scalar(repr) => {
                self.u8(0)?;
                self.scalar(*repr)
            }
            TypeShape::List(element) => {
                self.u8(1)?;
                self.ty(element, child_depth)
            }
            TypeShape::Map { key, value } => {
                self.u8(2)?;
                self.ty(key, child_depth)?;
                self.ty(value, child_depth)
            }
            TypeShape::Tuple(elements) => {
                self.u8(3)?;
                self.count(elements.len())?;
                self.ensure_node_capacity(elements.len())?;
                for element in elements {
                    self.ty(element, child_depth)?;
                }
                Ok(())
            }
            TypeShape::Record(fields) => {
                self.u8(4)?;
                self.count(fields.len())?;
                self.ensure_node_capacity(fields.len())?;
                for field in fields {
                    self.node(child_depth)?;
                    self.string(field.name())?;
                    self.bool(matches!(field.presence(), Presence::Optional))?;
                    self.ty(field.ty(), child_depth)?;
                }
                Ok(())
            }
            _ => Err(unsupported("unknown type shape variant")),
        }
    }

    fn scalar(&mut self, repr: ScalarRepr) -> Result<()> {
        let (tag, bits) = match repr {
            ScalarRepr::Bool => (0, None),
            ScalarRepr::Truth => (1, None),
            ScalarRepr::Int { bits } => (2, Some(bits)),
            ScalarRepr::UInt { bits } => (3, Some(bits)),
            ScalarRepr::Float32 => (4, None),
            ScalarRepr::Float64 => (5, None),
            ScalarRepr::Decimal => (6, None),
            ScalarRepr::Char => (7, None),
            ScalarRepr::String => (8, None),
            ScalarRepr::Bytes => (9, None),
            ScalarRepr::Uuid => (10, None),
            ScalarRepr::Date => (11, None),
            ScalarRepr::Time => (12, None),
            ScalarRepr::LocalDateTime => (13, None),
            ScalarRepr::Instant => (14, None),
            ScalarRepr::Duration => (15, None),
            _ => return Err(unsupported("unknown scalar representation")),
        };
        self.u8(tag)?;
        if let Some(bits) = bits {
            self.u8(bits)?;
        }
        Ok(())
    }

    fn datum(&mut self, datum: &Datum, depth: usize) -> Result<()> {
        self.node(depth)?;
        match datum {
            Datum::Missing => self.u8(0),
            Datum::Null => self.u8(1),
            Datum::Value(value) => {
                self.u8(2)?;
                self.value(value, depth)
            }
        }
    }

    fn value(&mut self, value: &Value, depth: usize) -> Result<()> {
        let child_depth = depth.saturating_add(1);
        match value {
            Value::Bool(value) => {
                self.u8(0)?;
                self.bool(*value)
            }
            Value::Truth(value) => {
                self.u8(1)?;
                self.u8(match value {
                    Truth::False => 0,
                    Truth::True => 1,
                    Truth::Unknown => 2,
                })
            }
            Value::Int(value) => {
                self.u8(2)?;
                self.i128(*value)
            }
            Value::UInt(value) => {
                self.u8(3)?;
                self.u128(*value)
            }
            Value::Float32(value) => {
                self.u8(4)?;
                self.u32(value.to_bits())
            }
            Value::Float64(value) => {
                self.u8(5)?;
                self.u64(value.to_bits())
            }
            Value::Decimal(value) => {
                let normalized = value.normalize();
                self.u8(6)?;
                self.i128(normalized.mantissa())?;
                self.u32(normalized.scale())
            }
            Value::Char(value) => {
                self.u8(7)?;
                self.u32(u32::from(*value))
            }
            Value::String(value) => {
                self.u8(8)?;
                self.string(value)
            }
            Value::Bytes(value) => {
                self.u8(9)?;
                self.literal(value)
            }
            Value::Uuid(value) => {
                self.u8(10)?;
                self.write(value.as_bytes())
            }
            Value::Date(value) => {
                self.u8(15)?;
                self.i64(i64::from(value.to_julian_day()))
            }
            Value::Time(value) => {
                self.u8(16)?;
                self.u8(value.hour())?;
                self.u8(value.minute())?;
                self.u8(value.second())?;
                self.u32(value.nanosecond())
            }
            Value::LocalDateTime(value) => {
                self.u8(17)?;
                self.i64(i64::from(value.date().to_julian_day()))?;
                self.u8(value.time().hour())?;
                self.u8(value.time().minute())?;
                self.u8(value.time().second())?;
                self.u32(value.time().nanosecond())
            }
            Value::Instant(value) => {
                self.u8(18)?;
                self.i128(value.unix_timestamp_nanos())
            }
            Value::Duration(value) => {
                self.u8(19)?;
                self.i128(value.whole_nanoseconds())
            }
            Value::List(values) => {
                self.u8(11)?;
                self.datum_list(values, child_depth)
            }
            Value::Map(entries) => {
                self.u8(12)?;
                self.map(entries, child_depth)
            }
            Value::Tuple(values) => {
                self.u8(13)?;
                self.datum_list(values, child_depth)
            }
            Value::Record(fields) => {
                self.u8(14)?;
                self.count(fields.len())?;
                self.ensure_node_capacity(fields.len())?;
                for (name, datum) in fields {
                    self.string(name)?;
                    self.datum(datum, child_depth)?;
                }
                Ok(())
            }
            _ => Err(unsupported("unknown logical value variant")),
        }
    }

    fn datum_list(&mut self, values: &[Datum], depth: usize) -> Result<()> {
        self.count(values.len())?;
        self.ensure_node_capacity(values.len())?;
        for value in values {
            self.datum(value, depth)?;
        }
        Ok(())
    }

    fn map(&mut self, entries: &[(Datum, Datum)], depth: usize) -> Result<()> {
        self.count(entries.len())?;
        let datum_count = entries
            .len()
            .checked_mul(2)
            .ok_or_else(|| limit_error("map node count overflow"))?;
        self.ensure_node_capacity(datum_count)?;

        let mut ordered = Vec::with_capacity(entries.len());
        let mut temporary_bytes = 0usize;
        for (key, value) in entries {
            let mut key_encoder = Encoder::new(self.limits)?;
            key_encoder.datum(key, depth)?;
            temporary_bytes = temporary_bytes
                .checked_add(key_encoder.output.len())
                .ok_or_else(|| limit_error("canonical map key size overflow"))?;
            if temporary_bytes > self.max_payload_bytes {
                return Err(limit_error("canonical map keys exceed max_total_bytes"));
            }
            ordered.push((key_encoder.output, key, value));
        }
        ordered.sort_by(|left, right| left.0.cmp(&right.0));
        for (_, key, value) in ordered {
            self.datum(key, depth)?;
            self.datum(value, depth)?;
        }
        Ok(())
    }

    fn model(&mut self, model: &ModelDef, depth: usize) -> Result<()> {
        self.node(depth)?;
        self.string(model.key().as_str())?;
        self.string(model.name())?;
        self.count(model.fields().len())?;
        self.ensure_node_capacity(model.fields().len())?;
        for field in model.fields() {
            self.node(depth.saturating_add(1))?;
            self.string(field.key().as_str())?;
            self.string(field.name())?;
            self.bool(matches!(field.presence(), Presence::Optional))?;
            validate_type_for_encode(field.ty(), self.limits)?;
            self.ty(field.ty(), depth.saturating_add(1))?;
        }
        match model.identity() {
            None => self.u8(0)?,
            Some(identity) => {
                self.u8(1)?;
                self.string_keys(identity.fields().iter().map(|key| key.as_str()), depth)?;
            }
        }
        self.count(model.unique_constraints().len())?;
        self.ensure_node_capacity(model.unique_constraints().len())?;
        for unique in model.unique_constraints() {
            self.node(depth.saturating_add(1))?;
            self.string_keys(unique.fields().iter().map(|key| key.as_str()), depth)?;
        }
        self.count(model.relations().len())?;
        self.ensure_node_capacity(model.relations().len())?;
        for relation in model.relations() {
            self.node(depth.saturating_add(1))?;
            self.string(relation.key().as_str())?;
            self.string(relation.name())?;
            self.string(relation.target_model().as_str())?;
            self.u8(match relation.cardinality() {
                Cardinality::One => 0,
                Cardinality::OptionalOne => 1,
                Cardinality::Many => 2,
            })?;
            self.count(relation.fields().len())?;
            self.ensure_node_capacity(relation.fields().len())?;
            for pair in relation.fields() {
                self.node(depth.saturating_add(2))?;
                self.string(pair.source().as_str())?;
                self.string(pair.target().as_str())?;
            }
        }
        self.count(model.references().len())?;
        self.ensure_node_capacity(model.references().len())?;
        for reference in model.references() {
            self.node(depth.saturating_add(1))?;
            self.string_keys(
                reference.source_fields().iter().map(|key| key.as_str()),
                depth,
            )?;
            self.string(reference.target_model().as_str())?;
            self.string_keys(
                reference.target_fields().iter().map(|key| key.as_str()),
                depth,
            )?;
        }
        Ok(())
    }

    fn string_keys<'a>(
        &mut self,
        keys: impl ExactSizeIterator<Item = &'a str>,
        depth: usize,
    ) -> Result<()> {
        self.count(keys.len())?;
        self.ensure_node_capacity(keys.len())?;
        for key in keys {
            self.node(depth.saturating_add(2))?;
            self.string(key)?;
        }
        Ok(())
    }
}

fn limit_error(message: &'static str) -> WireError {
    WireError::new(WireErrorKind::LimitExceeded, message)
}

fn unsupported(message: &'static str) -> WireError {
    WireError::new(WireErrorKind::UnsupportedValue, message)
}
