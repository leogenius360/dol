//! Explicit byte-string value distinct from `Vec<u8>` list semantics.

use crate::diagnostic::{Diagnostic, Result};
use crate::types::{DataType, ScalarRepr, ScalarType, TypeDef};

use super::{DataValue, Datum, DatumRef, Value, ValueRef};

/// Opaque byte string. `Vec<u8>` remains a list of unsigned integers.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Bytes(Vec<u8>);

impl Bytes {
    /// Creates a byte string.
    #[must_use]
    pub fn new(bytes: impl Into<Vec<u8>>) -> Self {
        Self(bytes.into())
    }

    /// Borrows bytes.
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }

    /// Consumes the wrapper.
    #[must_use]
    pub fn into_vec(self) -> Vec<u8> {
        self.0
    }
}

impl DataType for Bytes {
    fn type_def() -> TypeDef {
        TypeDef::scalar("dol/bytes", 1, ScalarRepr::Bytes)
    }
}

impl ScalarType for Bytes {}

impl DataValue for Bytes {
    fn datum_ref(&self) -> DatumRef<'_> {
        DatumRef::Value(ValueRef::Bytes(&self.0))
    }

    fn from_datum(datum: Datum) -> Result<Self> {
        match datum {
            Datum::Value(Value::Bytes(value)) => Ok(Self(value)),
            _ => Err(Diagnostic::error(
                "VALUE-DECODE-006",
                "canonical DOL value does not match the requested Rust type",
            )),
        }
    }
}
