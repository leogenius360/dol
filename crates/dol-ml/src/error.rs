//! Structured errors for ML semantic validation and execution.

use core::fmt;

/// Result type returned by `dol-ml` operations.
pub type Result<T> = core::result::Result<T, MlError>;

/// Stable, redaction-safe ML diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MlError {
    code: &'static str,
    message: String,
}

impl MlError {
    /// Creates a diagnostic with a stable machine-readable code.
    #[must_use]
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// Stable machine-readable diagnostic code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        self.code
    }

    /// Redaction-safe human-readable explanation.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for MlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for MlError {}
