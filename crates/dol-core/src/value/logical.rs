use std::collections::BTreeMap;

use super::{Datum, DatumRef};

/// Borrowed sequence interface for first-class Rust collections.
pub trait SequenceView {
    /// Number of elements.
    fn len(&self) -> usize;

    /// Whether the sequence is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Borrows one element.
    fn get(&self, index: usize) -> Option<DatumRef<'_>>;
}

/// Borrowed sequence representation used by logical values.
///
/// Canonical owned DOL values store their elements as [`Datum`] slices, while
/// first-class Rust collections can expose themselves through [`SequenceView`].
/// Keeping both forms avoids allocating adapter objects merely to borrow a
/// collection.
#[derive(Clone, Copy)]
pub enum SequenceRef<'a> {
    /// Canonical DOL datum slice.
    Datums(&'a [Datum]),
    /// Arbitrary first-class Rust sequence view.
    View(&'a dyn SequenceView),
}

impl SequenceRef<'_> {
    /// Number of elements.
    #[must_use]
    pub fn len(&self) -> usize {
        match self {
            Self::Datums(values) => values.len(),
            Self::View(view) => view.len(),
        }
    }

    /// Whether the sequence is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Borrows one element.
    #[must_use]
    pub fn get(&self, index: usize) -> Option<DatumRef<'_>> {
        match self {
            Self::Datums(values) => values.get(index).map(Datum::as_borrowed),
            Self::View(view) => view.get(index),
        }
    }
}

/// Borrowed map interface for first-class Rust mappings.
pub trait MapView {
    /// Number of entries.
    fn len(&self) -> usize;

    /// Whether the map is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Borrows one key/value pair by deterministic iteration position.
    fn entry(&self, index: usize) -> Option<(DatumRef<'_>, DatumRef<'_>)>;
}

/// Borrowed map representation used by logical values.
///
/// Canonical DOL maps use a datum-pair slice. Arbitrary Rust mappings can
/// expose themselves through [`MapView`] without being converted into an
/// owned intermediate value.
#[derive(Clone, Copy)]
pub enum MapRef<'a> {
    /// Canonical DOL map entries.
    Entries(&'a [(Datum, Datum)]),
    /// Arbitrary first-class Rust map view.
    View(&'a dyn MapView),
}

impl MapRef<'_> {
    /// Number of entries.
    #[must_use]
    pub fn len(&self) -> usize {
        match self {
            Self::Entries(entries) => entries.len(),
            Self::View(view) => view.len(),
        }
    }

    /// Whether the map is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Borrows one key/value pair by deterministic iteration position.
    #[must_use]
    pub fn entry(&self, index: usize) -> Option<(DatumRef<'_>, DatumRef<'_>)> {
        match self {
            Self::Entries(entries) => entries
                .get(index)
                .map(|(key, value)| (key.as_borrowed(), value.as_borrowed())),
            Self::View(view) => view.entry(index),
        }
    }
}

/// Borrowed named-record interface for first-class structured Rust values.
pub trait RecordValueView {
    /// Number of fields exposed by the value.
    fn len(&self) -> usize;

    /// Whether the record is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Borrows one field name and datum by canonical field position.
    fn field(&self, index: usize) -> Option<(&str, DatumRef<'_>)>;
}

/// Owned backend-independent logical value.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Value {
    /// Boolean value.
    Bool(bool),
    /// Three-valued query truth.
    Truth(crate::semantics::Truth),
    /// Signed integer normalized to 128-bit storage.
    Int(i128),
    /// Unsigned integer normalized to 128-bit storage.
    UInt(u128),
    /// 32-bit floating-point value.
    Float32(f32),
    /// 64-bit floating-point value.
    Float64(f64),
    /// Exact decimal value.
    Decimal(rust_decimal::Decimal),
    /// Unicode scalar value.
    Char(char),
    /// UTF-8 string.
    String(String),
    /// Opaque bytes.
    Bytes(Vec<u8>),
    /// UUID value.
    Uuid(uuid::Uuid),
    /// Calendar date.
    Date(time::Date),
    /// Time of day.
    Time(time::Time),
    /// Local date and time without offset.
    LocalDateTime(time::PrimitiveDateTime),
    /// Absolute instant.
    Instant(time::OffsetDateTime),
    /// Signed elapsed duration.
    Duration(time::Duration),
    /// Ordered sequence.
    List(Vec<Datum>),
    /// Unordered unique key/value pairs.
    Map(Vec<(Datum, Datum)>),
    /// Ordered heterogeneous product.
    Tuple(Vec<Datum>),
    /// Named structured value.
    Record(BTreeMap<String, Datum>),
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Bool(left), Self::Bool(right)) => left == right,
            (Self::Truth(left), Self::Truth(right)) => left == right,
            (Self::Int(left), Self::Int(right)) => left == right,
            (Self::UInt(left), Self::UInt(right)) => left == right,
            (Self::Float32(left), Self::Float32(right)) => left == right,
            (Self::Float64(left), Self::Float64(right)) => left == right,
            (Self::Decimal(left), Self::Decimal(right)) => left == right,
            (Self::Char(left), Self::Char(right)) => left == right,
            (Self::String(left), Self::String(right)) => left == right,
            (Self::Bytes(left), Self::Bytes(right)) => left == right,
            (Self::Uuid(left), Self::Uuid(right)) => left == right,
            (Self::Date(left), Self::Date(right)) => left == right,
            (Self::Time(left), Self::Time(right)) => left == right,
            (Self::LocalDateTime(left), Self::LocalDateTime(right)) => left == right,
            (Self::Instant(left), Self::Instant(right)) => left == right,
            (Self::Duration(left), Self::Duration(right)) => left == right,
            (Self::List(left), Self::List(right)) | (Self::Tuple(left), Self::Tuple(right)) => {
                left == right
            }
            (Self::Map(left), Self::Map(right)) => maps_equal(left, right),
            (Self::Record(left), Self::Record(right)) => left == right,
            _ => false,
        }
    }
}

fn maps_equal(left: &[(Datum, Datum)], right: &[(Datum, Datum)]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .all(|left_entry| right.iter().any(|right_entry| left_entry == right_entry))
}

/// Borrowed view over a logical value.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub enum ValueRef<'a> {
    /// Boolean value.
    Bool(bool),
    /// Three-valued query truth.
    Truth(crate::semantics::Truth),
    /// Signed integer normalized to 128-bit storage.
    Int(i128),
    /// Unsigned integer normalized to 128-bit storage.
    UInt(u128),
    /// 32-bit floating-point value.
    Float32(f32),
    /// 64-bit floating-point value.
    Float64(f64),
    /// Exact decimal value.
    Decimal(rust_decimal::Decimal),
    /// Unicode scalar value.
    Char(char),
    /// Borrowed UTF-8 string.
    String(&'a str),
    /// Borrowed bytes.
    Bytes(&'a [u8]),
    /// UUID value.
    Uuid(uuid::Uuid),
    /// Calendar date.
    Date(time::Date),
    /// Time of day.
    Time(time::Time),
    /// Local date and time without offset.
    LocalDateTime(time::PrimitiveDateTime),
    /// Absolute instant.
    Instant(time::OffsetDateTime),
    /// Signed elapsed duration.
    Duration(time::Duration),
    /// Borrowed homogeneous sequence.
    List(SequenceRef<'a>),
    /// Borrowed map.
    Map(MapRef<'a>),
    /// Borrowed heterogeneous tuple.
    Tuple(SequenceRef<'a>),
    /// Borrowed named structured value.
    Record(&'a dyn RecordValueView),
}

impl core::fmt::Debug for ValueRef<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Bool(value) => f.debug_tuple("Bool").field(value).finish(),
            Self::Truth(value) => f.debug_tuple("Truth").field(value).finish(),
            Self::Int(value) => f.debug_tuple("Int").field(value).finish(),
            Self::UInt(value) => f.debug_tuple("UInt").field(value).finish(),
            Self::Float32(value) => f.debug_tuple("Float32").field(value).finish(),
            Self::Float64(value) => f.debug_tuple("Float64").field(value).finish(),
            Self::Decimal(value) => f.debug_tuple("Decimal").field(value).finish(),
            Self::Char(value) => f.debug_tuple("Char").field(value).finish(),
            Self::String(value) => f.debug_tuple("String").field(value).finish(),
            Self::Bytes(value) => f.debug_tuple("Bytes").field(value).finish(),
            Self::Uuid(value) => f.debug_tuple("Uuid").field(value).finish(),
            Self::Date(value) => f.debug_tuple("Date").field(value).finish(),
            Self::Time(value) => f.debug_tuple("Time").field(value).finish(),
            Self::LocalDateTime(value) => f.debug_tuple("LocalDateTime").field(value).finish(),
            Self::Instant(value) => f.debug_tuple("Instant").field(value).finish(),
            Self::Duration(value) => f.debug_tuple("Duration").field(value).finish(),
            Self::List(value) => f.debug_struct("List").field("len", &value.len()).finish(),
            Self::Map(value) => f.debug_struct("Map").field("len", &value.len()).finish(),
            Self::Tuple(value) => f.debug_struct("Tuple").field("len", &value.len()).finish(),
            Self::Record(value) => f.debug_struct("Record").field("len", &value.len()).finish(),
        }
    }
}

impl RecordValueView for BTreeMap<String, Datum> {
    fn len(&self) -> usize {
        BTreeMap::len(self)
    }

    fn field(&self, index: usize) -> Option<(&str, DatumRef<'_>)> {
        self.iter()
            .nth(index)
            .map(|(name, datum)| (name.as_str(), datum.as_borrowed()))
    }
}

impl Value {
    /// Approximate owned logical payload bytes used by bounded local execution.
    #[doc(hidden)]
    #[must_use]
    pub fn logical_bytes(&self) -> u64 {
        match self {
            Self::Bool(_) | Self::Truth(_) => 1,
            Self::Int(_) | Self::UInt(_) | Self::Decimal(_) => 16,
            Self::Float32(_) | Self::Char(_) | Self::Date(_) => 4,
            Self::Float64(_) | Self::Time(_) => 8,
            Self::Uuid(_) => 16,
            Self::LocalDateTime(_) | Self::Instant(_) | Self::Duration(_) => 16,
            Self::String(value) => 8_u64.saturating_add(value.len() as u64),
            Self::Bytes(value) => 8_u64.saturating_add(value.len() as u64),
            Self::List(values) | Self::Tuple(values) => values.iter().fold(8_u64, |size, datum| {
                size.saturating_add(datum.logical_bytes())
            }),
            Self::Map(entries) => entries.iter().fold(8_u64, |size, (key, value)| {
                size.saturating_add(key.logical_bytes())
                    .saturating_add(value.logical_bytes())
            }),
            Self::Record(fields) => fields.iter().fold(8_u64, |size, (name, datum)| {
                size.saturating_add(name.len() as u64)
                    .saturating_add(datum.logical_bytes())
            }),
        }
    }

    /// Returns a borrowed view.
    #[must_use]
    pub fn as_borrowed(&self) -> ValueRef<'_> {
        match self {
            Self::Bool(value) => ValueRef::Bool(*value),
            Self::Truth(value) => ValueRef::Truth(*value),
            Self::Int(value) => ValueRef::Int(*value),
            Self::UInt(value) => ValueRef::UInt(*value),
            Self::Float32(value) => ValueRef::Float32(*value),
            Self::Float64(value) => ValueRef::Float64(*value),
            Self::Decimal(value) => ValueRef::Decimal(*value),
            Self::Char(value) => ValueRef::Char(*value),
            Self::String(value) => ValueRef::String(value),
            Self::Bytes(value) => ValueRef::Bytes(value),
            Self::Uuid(value) => ValueRef::Uuid(*value),
            Self::Date(value) => ValueRef::Date(*value),
            Self::Time(value) => ValueRef::Time(*value),
            Self::LocalDateTime(value) => ValueRef::LocalDateTime(*value),
            Self::Instant(value) => ValueRef::Instant(*value),
            Self::Duration(value) => ValueRef::Duration(*value),
            Self::List(values) => ValueRef::List(SequenceRef::Datums(values.as_slice())),
            Self::Map(entries) => ValueRef::Map(MapRef::Entries(entries.as_slice())),
            Self::Tuple(values) => ValueRef::Tuple(SequenceRef::Datums(values.as_slice())),
            Self::Record(fields) => ValueRef::Record(fields),
        }
    }
}
