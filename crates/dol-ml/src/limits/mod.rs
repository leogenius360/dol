//! Explicit resource limits for ML values and operations.

use std::time::Duration;

use crate::{MlError, Result};

const DEFAULT_MAX_VECTOR_DIMENSIONS: usize = 16_384;
const DEFAULT_MAX_SEARCH_CANDIDATES: usize = 1_000_000;
const DEFAULT_MAX_SEARCH_RESULTS: usize = 10_000;
const DEFAULT_MAX_FEATURES: usize = 4_096;
const DEFAULT_MAX_TEXT_BYTES: usize = 4_096;

fn require_non_zero(name: &'static str, value: usize) -> Result<usize> {
    if value == 0 {
        return Err(MlError::new(
            "ML-LIMIT-001",
            format!("{name} must be greater than zero"),
        ));
    }
    Ok(value)
}

/// Limits applied while constructing dynamic vectors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VectorLimits {
    max_dimensions: usize,
}

impl VectorLimits {
    /// Creates vector limits.
    pub fn new(max_dimensions: usize) -> Result<Self> {
        Ok(Self {
            max_dimensions: require_non_zero("max vector dimensions", max_dimensions)?,
        })
    }

    /// Maximum accepted vector dimension.
    #[must_use]
    pub const fn max_dimensions(self) -> usize {
        self.max_dimensions
    }
}

impl Default for VectorLimits {
    fn default() -> Self {
        Self {
            max_dimensions: DEFAULT_MAX_VECTOR_DIMENSIONS,
        }
    }
}

/// Limits applied to vector search requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchLimits {
    max_candidates: usize,
    max_results: usize,
}

impl SearchLimits {
    /// Creates search limits.
    pub fn new(max_candidates: usize, max_results: usize) -> Result<Self> {
        Ok(Self {
            max_candidates: require_non_zero("max search candidates", max_candidates)?,
            max_results: require_non_zero("max search results", max_results)?,
        })
    }

    /// Maximum candidates one search may inspect.
    #[must_use]
    pub const fn max_candidates(self) -> usize {
        self.max_candidates
    }

    /// Maximum matches one search may return.
    #[must_use]
    pub const fn max_results(self) -> usize {
        self.max_results
    }
}

impl Default for SearchLimits {
    fn default() -> Self {
        Self {
            max_candidates: DEFAULT_MAX_SEARCH_CANDIDATES,
            max_results: DEFAULT_MAX_SEARCH_RESULTS,
        }
    }
}

/// Limits applied to named feature materialization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeatureLimits {
    max_features: usize,
    max_name_bytes: usize,
    max_category_bytes: usize,
}

impl FeatureLimits {
    /// Creates feature limits.
    pub fn new(
        max_features: usize,
        max_name_bytes: usize,
        max_category_bytes: usize,
    ) -> Result<Self> {
        Ok(Self {
            max_features: require_non_zero("max features", max_features)?,
            max_name_bytes: require_non_zero("max feature-name bytes", max_name_bytes)?,
            max_category_bytes: require_non_zero(
                "max categorical feature bytes",
                max_category_bytes,
            )?,
        })
    }

    /// Maximum entries in one feature set.
    #[must_use]
    pub const fn max_features(self) -> usize {
        self.max_features
    }

    /// Maximum UTF-8 bytes in a feature name.
    #[must_use]
    pub const fn max_name_bytes(self) -> usize {
        self.max_name_bytes
    }

    /// Maximum UTF-8 bytes in a categorical value.
    #[must_use]
    pub const fn max_category_bytes(self) -> usize {
        self.max_category_bytes
    }
}

impl Default for FeatureLimits {
    fn default() -> Self {
        Self {
            max_features: DEFAULT_MAX_FEATURES,
            max_name_bytes: 256,
            max_category_bytes: DEFAULT_MAX_TEXT_BYTES,
        }
    }
}

/// Limits applied to one provider inference call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InferenceLimits {
    max_batch_items: usize,
    max_output_items: usize,
    max_input_bytes: u64,
    max_cost_microunits: u64,
    timeout: Duration,
}

impl InferenceLimits {
    /// Creates inference limits.
    pub fn new(
        max_batch_items: usize,
        max_output_items: usize,
        max_input_bytes: u64,
        max_cost_microunits: u64,
        timeout: Duration,
    ) -> Result<Self> {
        if max_input_bytes == 0 || timeout.is_zero() {
            return Err(MlError::new(
                "ML-LIMIT-002",
                "inference byte limit and timeout must be greater than zero",
            ));
        }
        Ok(Self {
            max_batch_items: require_non_zero("max inference batch items", max_batch_items)?,
            max_output_items: require_non_zero("max inference output items", max_output_items)?,
            max_input_bytes,
            max_cost_microunits,
            timeout,
        })
    }

    /// Maximum items accepted in a single input batch.
    #[must_use]
    pub const fn max_batch_items(self) -> usize {
        self.max_batch_items
    }

    /// Maximum items returned by a provider.
    #[must_use]
    pub const fn max_output_items(self) -> usize {
        self.max_output_items
    }

    /// Maximum estimated encoded input bytes.
    #[must_use]
    pub const fn max_input_bytes(self) -> u64 {
        self.max_input_bytes
    }

    /// Maximum provider cost in provider-independent microunits.
    #[must_use]
    pub const fn max_cost_microunits(self) -> u64 {
        self.max_cost_microunits
    }

    /// Provider call deadline duration.
    #[must_use]
    pub const fn timeout(self) -> Duration {
        self.timeout
    }
}

impl Default for InferenceLimits {
    fn default() -> Self {
        Self {
            max_batch_items: 1_024,
            max_output_items: 1_024,
            max_input_bytes: 16 * 1024 * 1024,
            max_cost_microunits: 1_000_000,
            timeout: Duration::from_secs(30),
        }
    }
}

/// Limits applied to evaluation workloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EvaluationLimits {
    max_cases: usize,
}

impl EvaluationLimits {
    /// Creates evaluation limits.
    pub fn new(max_cases: usize) -> Result<Self> {
        Ok(Self {
            max_cases: require_non_zero("max evaluation cases", max_cases)?,
        })
    }

    /// Maximum cases accepted by one evaluation.
    #[must_use]
    pub const fn max_cases(self) -> usize {
        self.max_cases
    }
}

impl Default for EvaluationLimits {
    fn default() -> Self {
        Self { max_cases: 100_000 }
    }
}
