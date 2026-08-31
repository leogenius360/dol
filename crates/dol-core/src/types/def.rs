use core::fmt;
use std::collections::BTreeMap;

use crate::fingerprint::CanonicalHasher;

/// Stable semantic lineage identity of a DOL data type.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TypeKey(String);

impl TypeKey {
    /// Creates a semantic type lineage key.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the identifier text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for TypeKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("TypeKey").field(&self.0).finish()
    }
}

/// Exact 256-bit fingerprint of one canonical semantic type definition.
pub type TypeFingerprint = crate::fingerprint::Fingerprint;

/// Canonical scalar parameter attached to a semantic type definition.
///
/// Parameters let open Rust-first types describe semantic distinctions such as
/// decimal precision/scale, units, dimensions, or domain policies without
/// encoding those details into type-name strings.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum TypeParameter {
    /// Boolean parameter.
    Bool(bool),
    /// Signed integer parameter.
    Int(i64),
    /// Unsigned integer parameter.
    UInt(u64),
    /// UTF-8 text parameter.
    String(String),
}

impl From<bool> for TypeParameter {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<i64> for TypeParameter {
    fn from(value: i64) -> Self {
        Self::Int(value)
    }
}

impl From<u64> for TypeParameter {
    fn from(value: u64) -> Self {
        Self::UInt(value)
    }
}

impl From<String> for TypeParameter {
    fn from(value: String) -> Self {
        Self::String(value)
    }
}

impl From<&str> for TypeParameter {
    fn from(value: &str) -> Self {
        Self::String(value.to_owned())
    }
}

/// Whether explicit null is accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Nullability {
    /// A concrete value is required when the datum is present.
    NonNull,
    /// Explicit null is accepted.
    Nullable,
}

/// Whether a named field must be present.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Presence {
    /// The field must exist in the record.
    Required,
    /// The field may be missing from the record.
    Optional,
}

/// Named field inside a structured semantic value.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RecordField {
    name: String,
    ty: TypeDef,
    presence: Presence,
}

impl RecordField {
    /// Creates a required record field.
    #[must_use]
    pub fn required(name: impl Into<String>, ty: TypeDef) -> Self {
        Self {
            name: name.into(),
            ty,
            presence: Presence::Required,
        }
    }

    /// Creates an optional record field.
    #[must_use]
    pub fn optional(name: impl Into<String>, ty: TypeDef) -> Self {
        Self {
            name: name.into(),
            ty,
            presence: Presence::Optional,
        }
    }

    /// Field name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Field type.
    #[must_use]
    pub const fn ty(&self) -> &TypeDef {
        &self.ty
    }

    /// Field presence.
    #[must_use]
    pub const fn presence(&self) -> Presence {
        self.presence
    }
}

/// Semantic properties operations and constraints can rely on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TypeProperties {
    /// Values have defined equality semantics.
    pub equality: bool,
    /// Values support DOL ordered comparison semantics.
    pub ordering: bool,
    /// Values have stable hash/group-key semantics.
    pub keyable: bool,
    /// Values participate in numeric operations.
    pub numeric: bool,
    /// Values are integral numeric values.
    pub integral: bool,
    /// Values are exact rather than approximate numeric values.
    pub exact_numeric: bool,
    /// Values participate in temporal operations.
    pub temporal: bool,
    /// Values are collection-shaped.
    pub collection: bool,
}

impl TypeProperties {
    /// Empty capability set for specialized domain types.
    #[must_use]
    pub const fn none() -> Self {
        Self {
            equality: false,
            ordering: false,
            keyable: false,
            numeric: false,
            integral: false,
            exact_numeric: false,
            temporal: false,
            collection: false,
        }
    }
}

/// Canonical runtime representation of a scalar semantic type.
///
/// This is a closed representation vocabulary, not a closed semantic type
/// universe. Domain types may share representations while retaining distinct
/// semantic identities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ScalarRepr {
    /// Boolean value.
    Bool,
    /// Three-valued DOL truth used by conditional expressions.
    Truth,
    /// Signed integer of the declared bit width.
    Int { bits: u8 },
    /// Unsigned integer of the declared bit width.
    UInt { bits: u8 },
    /// IEEE-754 32-bit floating-point value.
    Float32,
    /// IEEE-754 64-bit floating-point value.
    Float64,
    /// Exact decimal value using DOL decimal semantics.
    Decimal,
    /// Unicode scalar value.
    Char,
    /// UTF-8 string value.
    String,
    /// Opaque bytes.
    Bytes,
    /// UUID value.
    Uuid,
    /// Calendar date.
    Date,
    /// Time of day without a date or offset.
    Time,
    /// Calendar date and time without a timezone or offset.
    LocalDateTime,
    /// Absolute point on the timeline.
    Instant,
    /// Signed elapsed duration.
    Duration,
}

/// Structural shape of a semantic type.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TypeShape {
    /// Atomic value represented by one canonical scalar form.
    Scalar(ScalarRepr),
    /// Ordered variable-length sequence.
    List(Box<TypeDef>),
    /// Unordered unique key/value mapping.
    Map {
        /// Key type.
        key: Box<TypeDef>,
        /// Value type.
        value: Box<TypeDef>,
    },
    /// Ordered heterogeneous product.
    Tuple(Vec<TypeDef>),
    /// Named structured value whose field declaration order is non-semantic.
    Record(Vec<RecordField>),
}

/// Canonical semantic type definition.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TypeDef {
    key: TypeKey,
    version: u32,
    shape: TypeShape,
    nullability: Nullability,
    properties: TypeProperties,
    parameters: BTreeMap<String, TypeParameter>,
}

impl TypeDef {
    /// Creates a non-null scalar semantic type with explicit canonical representation.
    #[must_use]
    pub fn scalar(key: impl Into<String>, version: u32, repr: ScalarRepr) -> Self {
        Self {
            key: TypeKey::new(key),
            version,
            shape: TypeShape::Scalar(repr),
            nullability: Nullability::NonNull,
            properties: scalar_properties(repr),
            parameters: BTreeMap::new(),
        }
    }

    /// Creates a canonical anonymous tuple type from ordered element semantics.
    ///
    /// Tuple lineage is content-addressed from the exact element definitions so
    /// independently-authored tuple projections converge on the same semantic type.
    #[must_use]
    pub fn tuple(elements: impl IntoIterator<Item = TypeDef>) -> Self {
        let elements = elements.into_iter().collect::<Vec<_>>();
        let mut hasher = CanonicalHasher::new(b"type/tuple-lineage/v1");
        hasher.u64(elements.len() as u64);
        for element in &elements {
            hasher.bytes(element.fingerprint().as_bytes());
        }
        let fingerprint = hasher.finish();
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut key = String::with_capacity(10 + fingerprint.as_bytes().len() * 2);
        key.push_str("dol/tuple/");
        for byte in fingerprint.as_bytes() {
            key.push(char::from(HEX[usize::from(*byte >> 4)]));
            key.push(char::from(HEX[usize::from(*byte & 0x0f)]));
        }
        Self::shaped(key, 1, TypeShape::Tuple(elements))
    }

    /// Creates a semantic type with explicit structural shape.
    #[must_use]
    pub fn shaped(key: impl Into<String>, version: u32, shape: TypeShape) -> Self {
        let properties = shape_properties(&shape);
        let mut definition = Self {
            key: TypeKey::new(key),
            version,
            shape,
            nullability: Nullability::NonNull,
            properties,
            parameters: BTreeMap::new(),
        };
        definition.canonicalize();
        definition
    }

    /// Replaces the semantic property set for this type.
    ///
    /// This is primarily useful for domain types whose canonical runtime
    /// representation supports more operations than the domain semantics do.
    #[must_use]
    pub fn with_properties(mut self, properties: TypeProperties) -> Self {
        self.properties = properties;
        self
    }

    /// Adds or replaces one canonical semantic type parameter.
    #[must_use]
    pub fn parameter(mut self, name: impl Into<String>, value: impl Into<TypeParameter>) -> Self {
        self.parameters.insert(name.into(), value.into());
        self
    }

    /// Returns a nullable copy.
    #[must_use]
    pub fn nullable(mut self) -> Self {
        self.nullability = Nullability::Nullable;
        self
    }

    /// Returns a non-null copy.
    #[must_use]
    pub fn non_null(mut self) -> Self {
        self.nullability = Nullability::NonNull;
        self
    }

    /// Semantic lineage identity.
    #[must_use]
    pub const fn key(&self) -> &TypeKey {
        &self.key
    }

    /// Exact fingerprint of this canonical semantic definition.
    #[must_use]
    pub fn fingerprint(&self) -> TypeFingerprint {
        crate::fingerprint::fingerprint_type_def(self)
    }

    /// Semantic version.
    #[must_use]
    pub const fn version(&self) -> u32 {
        self.version
    }

    /// Structural shape.
    #[must_use]
    pub const fn shape(&self) -> &TypeShape {
        &self.shape
    }

    /// Nullability.
    #[must_use]
    pub const fn nullability(&self) -> Nullability {
        self.nullability
    }

    /// Semantic type properties.
    #[must_use]
    pub const fn properties(&self) -> TypeProperties {
        self.properties
    }

    /// Canonical semantic parameters in key order.
    #[must_use]
    pub const fn parameters(&self) -> &BTreeMap<String, TypeParameter> {
        &self.parameters
    }

    /// Whether explicit null is accepted.
    #[must_use]
    pub const fn is_nullable(&self) -> bool {
        matches!(self.nullability, Nullability::Nullable)
    }

    /// Whether two field types share the same non-null value semantics.
    #[must_use]
    pub fn same_value_type(&self, other: &Self) -> bool {
        self.key == other.key
            && self.version == other.version
            && self.shape == other.shape
            && self.properties == other.properties
            && self.parameters == other.parameters
    }

    pub(crate) fn canonicalize(&mut self) {
        match &mut self.shape {
            TypeShape::Scalar(_) => {}
            TypeShape::List(element) => element.canonicalize(),
            TypeShape::Map { key, value } => {
                key.canonicalize();
                value.canonicalize();
            }
            TypeShape::Tuple(elements) => {
                for element in elements {
                    element.canonicalize();
                }
            }
            TypeShape::Record(fields) => {
                for field in fields.iter_mut() {
                    field.ty.canonicalize();
                }
                fields.sort_by(|left, right| left.name.cmp(&right.name));
            }
        }
    }
}

fn scalar_properties(repr: ScalarRepr) -> TypeProperties {
    let mut properties = TypeProperties {
        equality: true,
        ordering: true,
        keyable: true,
        numeric: false,
        integral: false,
        exact_numeric: false,
        temporal: false,
        collection: false,
    };

    match repr {
        ScalarRepr::Int { .. } | ScalarRepr::UInt { .. } => {
            properties.numeric = true;
            properties.integral = true;
            properties.exact_numeric = true;
        }
        ScalarRepr::Float32 | ScalarRepr::Float64 => {
            properties.numeric = true;
            // Phase 2 defines IEEE comparison behavior; grouping/key equivalence remains deferred.
            properties.ordering = true;
            properties.keyable = false;
        }
        ScalarRepr::Decimal => {
            properties.numeric = true;
            properties.exact_numeric = true;
        }
        ScalarRepr::Truth => {
            properties.ordering = false;
        }
        ScalarRepr::Date
        | ScalarRepr::Time
        | ScalarRepr::LocalDateTime
        | ScalarRepr::Instant
        | ScalarRepr::Duration => {
            properties.temporal = true;
        }
        ScalarRepr::Bool
        | ScalarRepr::Char
        | ScalarRepr::String
        | ScalarRepr::Bytes
        | ScalarRepr::Uuid => {}
    }

    properties
}

fn shape_properties(shape: &TypeShape) -> TypeProperties {
    let (equality, keyable) = match shape {
        TypeShape::Scalar(repr) => return scalar_properties(*repr),
        TypeShape::List(element) => (element.properties().equality, false),
        TypeShape::Map { key, value } => (
            key.properties().equality && value.properties().equality,
            false,
        ),
        TypeShape::Tuple(elements) => (
            elements.iter().all(|element| element.properties().equality),
            elements.iter().all(|element| element.properties().keyable),
        ),
        TypeShape::Record(fields) => (
            fields.iter().all(|field| field.ty().properties().equality),
            fields.iter().all(|field| field.ty().properties().keyable),
        ),
    };

    TypeProperties {
        equality,
        ordering: false,
        keyable,
        numeric: false,
        integral: false,
        exact_numeric: false,
        temporal: false,
        collection: true,
    }
}
