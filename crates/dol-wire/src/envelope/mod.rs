//! Versioned interchange envelope.

use crate::error::{Result, WireError, WireErrorKind};

/// Fixed frame prefix.
pub const MAGIC: [u8; 4] = *b"DOLW";
/// Current protocol major version.
pub const MAJOR_VERSION: u16 = 1;
/// Current protocol minor version.
pub const MINOR_VERSION: u16 = 0;
/// Bytes in a complete frame header.
pub const HEADER_LEN: usize = 20;

/// Semantic payload carried by an envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PayloadKind {
    /// A type definition.
    TypeDef,
    /// A type definition, presence contract, and datum.
    TypedDatum,
    /// One model definition.
    ModelDef,
    /// A mutually-resolvable model set.
    ModelSet,
}

impl PayloadKind {
    pub(crate) const fn tag(self) -> u8 {
        match self {
            Self::TypeDef => 1,
            Self::TypedDatum => 2,
            Self::ModelDef => 3,
            Self::ModelSet => 4,
        }
    }

    pub(crate) fn from_tag(tag: u8, offset: usize) -> Result<Self> {
        match tag {
            1 => Ok(Self::TypeDef),
            2 => Ok(Self::TypedDatum),
            3 => Ok(Self::ModelDef),
            4 => Ok(Self::ModelSet),
            _ => Err(WireError::at(
                WireErrorKind::UnsupportedPayload,
                offset,
                format!("unknown payload kind {tag}"),
            )),
        }
    }
}

/// Parsed wire envelope whose payload remains an untrusted DTO byte sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope {
    major_version: u16,
    minor_version: u16,
    kind: PayloadKind,
    payload: Vec<u8>,
}

impl Envelope {
    /// Creates a current-version envelope around encoded DTO bytes.
    #[must_use]
    pub fn new(kind: PayloadKind, payload: Vec<u8>) -> Self {
        Self {
            major_version: MAJOR_VERSION,
            minor_version: MINOR_VERSION,
            kind,
            payload,
        }
    }

    pub(crate) const fn decoded(
        major_version: u16,
        minor_version: u16,
        kind: PayloadKind,
        payload: Vec<u8>,
    ) -> Self {
        Self {
            major_version,
            minor_version,
            kind,
            payload,
        }
    }

    /// Protocol major version.
    #[must_use]
    pub const fn major_version(&self) -> u16 {
        self.major_version
    }

    /// Protocol minor version.
    #[must_use]
    pub const fn minor_version(&self) -> u16 {
        self.minor_version
    }

    /// Declared payload kind.
    #[must_use]
    pub const fn kind(&self) -> PayloadKind {
        self.kind
    }

    /// Still-untrusted payload bytes.
    #[must_use]
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    /// Consumes the envelope and returns its payload.
    #[must_use]
    pub fn into_payload(self) -> Vec<u8> {
        self.payload
    }
}
