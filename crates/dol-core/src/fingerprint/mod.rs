//! Stable semantic fingerprints.

use core::fmt;

/// A 256-bit semantic fingerprint.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Fingerprint([u8; 32]);

impl Fingerprint {
    /// Creates a fingerprint from digest bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Returns the digest bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Fingerprint(")?;
        for byte in &self.0[..8] {
            write!(f, "{byte:02x}")?;
        }
        write!(f, "…)")
    }
}

pub(crate) struct CanonicalHasher(blake3::Hasher);

impl CanonicalHasher {
    pub(crate) fn new(domain: &'static [u8]) -> Self {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"DOL\0");
        hasher.update(&(domain.len() as u64).to_le_bytes());
        hasher.update(domain);
        Self(hasher)
    }

    pub(crate) fn bytes(&mut self, value: &[u8]) {
        self.0.update(&(value.len() as u64).to_le_bytes());
        self.0.update(value);
    }

    pub(crate) fn str(&mut self, value: &str) {
        self.bytes(value.as_bytes());
    }

    pub(crate) fn u8(&mut self, value: u8) {
        self.0.update(&[value]);
    }

    pub(crate) fn u32(&mut self, value: u32) {
        self.0.update(&value.to_le_bytes());
    }

    pub(crate) fn u64(&mut self, value: u64) {
        self.0.update(&value.to_le_bytes());
    }

    pub(crate) fn finish(self) -> Fingerprint {
        Fingerprint(*self.0.finalize().as_bytes())
    }
}

/// Computes the canonical fingerprint of one exact semantic type definition.
pub(crate) fn fingerprint_type_def(ty: &crate::types::TypeDef) -> Fingerprint {
    let mut hasher = CanonicalHasher::new(b"type/v1");
    hash_type_def(&mut hasher, ty);
    hasher.finish()
}

pub(crate) fn hash_type_def(hasher: &mut CanonicalHasher, ty: &crate::types::TypeDef) {
    use crate::types::{Nullability, Presence, ScalarRepr, TypeParameter, TypeShape};

    hasher.str(ty.key().as_str());
    hasher.u32(ty.version());
    hasher.u8(match ty.nullability() {
        Nullability::NonNull => 0,
        Nullability::Nullable => 1,
    });

    let properties = ty.properties();
    hasher.u8(u8::from(properties.equality));
    hasher.u8(u8::from(properties.ordering));
    hasher.u8(u8::from(properties.keyable));
    hasher.u8(u8::from(properties.numeric));
    hasher.u8(u8::from(properties.integral));
    hasher.u8(u8::from(properties.exact_numeric));
    hasher.u8(u8::from(properties.temporal));
    hasher.u8(u8::from(properties.collection));

    hasher.u64(ty.parameters().len() as u64);
    for (name, value) in ty.parameters() {
        hasher.str(name);
        match value {
            TypeParameter::Bool(value) => {
                hasher.u8(0);
                hasher.u8(u8::from(*value));
            }
            TypeParameter::Int(value) => {
                hasher.u8(1);
                hasher.bytes(&value.to_le_bytes());
            }
            TypeParameter::UInt(value) => {
                hasher.u8(2);
                hasher.u64(*value);
            }
            TypeParameter::String(value) => {
                hasher.u8(3);
                hasher.str(value);
            }
        }
    }

    match ty.shape() {
        TypeShape::Scalar(repr) => {
            hasher.u8(0);
            match repr {
                ScalarRepr::Bool => hasher.u8(0),
                ScalarRepr::Int { bits } => {
                    hasher.u8(1);
                    hasher.u8(*bits);
                }
                ScalarRepr::UInt { bits } => {
                    hasher.u8(2);
                    hasher.u8(*bits);
                }
                ScalarRepr::Float32 => hasher.u8(3),
                ScalarRepr::Float64 => hasher.u8(4),
                ScalarRepr::Decimal => hasher.u8(5),
                ScalarRepr::Char => hasher.u8(6),
                ScalarRepr::String => hasher.u8(7),
                ScalarRepr::Bytes => hasher.u8(8),
                ScalarRepr::Uuid => hasher.u8(9),
                ScalarRepr::Date => hasher.u8(10),
                ScalarRepr::Time => hasher.u8(11),
                ScalarRepr::LocalDateTime => hasher.u8(12),
                ScalarRepr::Instant => hasher.u8(13),
                ScalarRepr::Duration => hasher.u8(14),
                ScalarRepr::Truth => hasher.u8(15),
            }
        }
        TypeShape::List(element) => {
            hasher.u8(1);
            hash_type_def(hasher, element);
        }
        TypeShape::Map { key, value } => {
            hasher.u8(2);
            hash_type_def(hasher, key);
            hash_type_def(hasher, value);
        }
        TypeShape::Tuple(elements) => {
            hasher.u8(3);
            hasher.u64(elements.len() as u64);
            for element in elements {
                hash_type_def(hasher, element);
            }
        }
        TypeShape::Record(fields) => {
            hasher.u8(4);
            hasher.u64(fields.len() as u64);
            for field in fields {
                hasher.str(field.name());
                hasher.u8(match field.presence() {
                    Presence::Required => 0,
                    Presence::Optional => 1,
                });
                hash_type_def(hasher, field.ty());
            }
        }
    }
}

/// Computes the canonical fingerprint of a typed DOL datum.
///
/// The semantic type definition is part of the fingerprint, so domain types
/// that share one runtime representation remain distinct. Floating NaNs,
/// signed zero, decimals, maps, and structured values use DOL canonical value
/// semantics rather than Rust `Debug` or display formatting.
pub fn fingerprint_datum(
    ty: &crate::types::TypeDef,
    datum: &crate::value::Datum,
) -> crate::diagnostic::Result<Fingerprint> {
    crate::value::validate_datum(ty, crate::types::Presence::Optional, datum)?;
    let mut hasher = CanonicalHasher::new(b"datum/v1");
    hash_type_def(&mut hasher, ty);
    hash_datum(&mut hasher, datum);
    Ok(hasher.finish())
}

pub(crate) fn hash_datum(hasher: &mut CanonicalHasher, datum: &crate::value::Datum) {
    use crate::value::Datum;

    match datum {
        Datum::Missing => hasher.u8(0),
        Datum::Null => hasher.u8(1),
        Datum::Value(value) => {
            hasher.u8(2);
            hash_value(hasher, value);
        }
    }
}

fn hash_value(hasher: &mut CanonicalHasher, value: &crate::value::Value) {
    use crate::value::Value;

    match value {
        Value::Bool(value) => {
            hasher.u8(0);
            hasher.u8(u8::from(*value));
        }
        Value::Truth(value) => {
            hasher.u8(1);
            hasher.u8(match value {
                crate::semantics::Truth::True => 0,
                crate::semantics::Truth::False => 1,
                crate::semantics::Truth::Unknown => 2,
            });
        }
        Value::Int(value) => {
            hasher.u8(2);
            hasher.bytes(&value.to_le_bytes());
        }
        Value::UInt(value) => {
            hasher.u8(3);
            hasher.bytes(&value.to_le_bytes());
        }
        Value::Float32(value) => {
            hasher.u8(4);
            hasher.u32(canonical_f32_bits(*value));
        }
        Value::Float64(value) => {
            hasher.u8(5);
            hasher.u64(canonical_f64_bits(*value));
        }
        Value::Decimal(value) => {
            hasher.u8(6);
            let normalized = value.normalize();
            hasher.bytes(&normalized.mantissa().to_le_bytes());
            hasher.u32(normalized.scale());
        }
        Value::Char(value) => {
            hasher.u8(7);
            hasher.u32(u32::from(*value));
        }
        Value::String(value) => {
            hasher.u8(8);
            hasher.str(value);
        }
        Value::Bytes(value) => {
            hasher.u8(9);
            hasher.bytes(value);
        }
        Value::Uuid(value) => {
            hasher.u8(10);
            hasher.bytes(value.as_bytes());
        }
        Value::Date(value) => {
            hasher.u8(11);
            hasher.bytes(&value.to_julian_day().to_le_bytes());
        }
        Value::Time(value) => {
            hasher.u8(12);
            hasher.u8(value.hour());
            hasher.u8(value.minute());
            hasher.u8(value.second());
            hasher.u32(value.nanosecond());
        }
        Value::LocalDateTime(value) => {
            hasher.u8(13);
            hasher.bytes(&value.date().to_julian_day().to_le_bytes());
            hasher.u8(value.time().hour());
            hasher.u8(value.time().minute());
            hasher.u8(value.time().second());
            hasher.u32(value.time().nanosecond());
        }
        Value::Instant(value) => {
            hasher.u8(14);
            hasher.bytes(&value.unix_timestamp_nanos().to_le_bytes());
        }
        Value::Duration(value) => {
            hasher.u8(15);
            hasher.bytes(&value.whole_nanoseconds().to_le_bytes());
        }
        Value::List(values) => {
            hasher.u8(16);
            hash_datums(hasher, values);
        }
        Value::Map(entries) => {
            hasher.u8(17);
            hash_map(hasher, entries);
        }
        Value::Tuple(values) => {
            hasher.u8(18);
            hash_datums(hasher, values);
        }
        Value::Record(fields) => {
            hasher.u8(19);
            hasher.u64(fields.len() as u64);
            for (name, value) in fields {
                hasher.str(name);
                hash_datum(hasher, value);
            }
        }
    }
}

fn hash_datums(hasher: &mut CanonicalHasher, values: &[crate::value::Datum]) {
    hasher.u64(values.len() as u64);
    for value in values {
        hash_datum(hasher, value);
    }
}

fn hash_map(hasher: &mut CanonicalHasher, entries: &[(crate::value::Datum, crate::value::Datum)]) {
    let mut entry_fingerprints = entries
        .iter()
        .map(|(key, value)| {
            let mut entry = CanonicalHasher::new(b"datum/map-entry/v1");
            hash_datum(&mut entry, key);
            hash_datum(&mut entry, value);
            entry.finish()
        })
        .collect::<Vec<_>>();
    entry_fingerprints.sort();
    hasher.u64(entry_fingerprints.len() as u64);
    for fingerprint in entry_fingerprints {
        hasher.bytes(fingerprint.as_bytes());
    }
}

fn canonical_f32_bits(value: f32) -> u32 {
    if value.is_nan() {
        0x7fc0_0000
    } else if value == 0.0 {
        0.0_f32.to_bits()
    } else {
        value.to_bits()
    }
}

fn canonical_f64_bits(value: f64) -> u64 {
    if value.is_nan() {
        0x7ff8_0000_0000_0000
    } else if value == 0.0 {
        0.0_f64.to_bits()
    } else {
        value.to_bits()
    }
}
