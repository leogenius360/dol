use super::{Value, ValueRef};

/// Owned field state distinguishing missing, null, and concrete values.
#[derive(Debug, Clone, PartialEq)]
pub enum Datum {
    /// Field is absent from the record.
    Missing,
    /// Field is present with explicit null.
    Null,
    /// Field is present with a concrete value.
    Value(Value),
}

/// Borrowed field state.
#[derive(Debug, Clone, Copy)]
pub enum DatumRef<'a> {
    /// Field is absent from the record.
    Missing,
    /// Field is present with explicit null.
    Null,
    /// Field is present with a borrowed concrete value.
    Value(ValueRef<'a>),
}

impl Datum {
    /// Approximate owned logical payload bytes used by bounded local execution.
    #[doc(hidden)]
    #[must_use]
    pub fn logical_bytes(&self) -> u64 {
        match self {
            Self::Missing | Self::Null => 1,
            Self::Value(value) => 1_u64.saturating_add(value.logical_bytes()),
        }
    }

    /// Returns a borrowed view.
    #[must_use]
    pub fn as_borrowed(&self) -> DatumRef<'_> {
        match self {
            Self::Missing => DatumRef::Missing,
            Self::Null => DatumRef::Null,
            Self::Value(value) => DatumRef::Value(value.as_borrowed()),
        }
    }
}
