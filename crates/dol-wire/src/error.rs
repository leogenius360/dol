//! Stable errors produced at the untrusted wire boundary.

use core::fmt;

/// Broad category of a wire failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum WireErrorKind {
    /// A configured resource limit was exceeded.
    LimitExceeded,
    /// The frame does not start with the DOL wire magic bytes.
    InvalidMagic,
    /// The frame uses an unsupported protocol version.
    UnsupportedVersion,
    /// The frame declares an unknown payload kind.
    UnsupportedPayload,
    /// A reserved header field or flag is non-zero.
    InvalidHeader,
    /// The declared payload length does not match the frame.
    LengthMismatch,
    /// Input ended before a complete value was available.
    UnexpectedEof,
    /// A tagged union contained an unknown or invalid tag.
    InvalidTag,
    /// Text was not valid UTF-8.
    InvalidUtf8,
    /// A decoded DTO is structurally or semantically invalid.
    InvalidValue,
    /// A valid value was encoded in a non-canonical form.
    NonCanonical,
    /// The current protocol cannot represent a supplied value.
    UnsupportedValue,
}

/// Error returned by wire encoding, decoding, validation, or lowering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireError {
    kind: WireErrorKind,
    offset: Option<usize>,
    message: String,
}

impl WireError {
    pub(crate) fn new(kind: WireErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            offset: None,
            message: message.into(),
        }
    }

    pub(crate) fn at(kind: WireErrorKind, offset: usize, message: impl Into<String>) -> Self {
        Self {
            kind,
            offset: Some(offset),
            message: message.into(),
        }
    }

    /// Error category.
    #[must_use]
    pub const fn kind(&self) -> WireErrorKind {
        self.kind
    }

    /// Byte offset associated with the failure, when available.
    #[must_use]
    pub const fn offset(&self) -> Option<usize> {
        self.offset
    }

    /// Human-readable failure description.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for WireError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(offset) = self.offset {
            write!(formatter, "wire error at byte {offset}: {}", self.message)
        } else {
            formatter.write_str(&self.message)
        }
    }
}

impl std::error::Error for WireError {}

/// Result returned by DOL wire operations.
pub type Result<T> = core::result::Result<T, WireError>;
