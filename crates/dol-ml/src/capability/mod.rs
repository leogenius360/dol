//! Exact ML capability claims and bounded placement decisions.

use dol_engine::{PlacementPolicy, Support};

use crate::vector::Distance;
use crate::{MlError, Result};

/// Engine/provider ML capability descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MlCapabilities {
    /// Exact exhaustive vector search.
    pub exact_vector_search: Support,
    /// Approximate vector search with explicit quality/work targets.
    pub approximate_vector_search: Support,
    /// Embedding generation.
    pub embedding: Support,
    /// General model inference.
    pub inference: Support,
    max_vector_dimensions: usize,
    vector_metrics: Vec<Distance>,
}

impl MlCapabilities {
    /// Creates an unsupported-by-default capability set.
    #[must_use]
    pub fn none() -> Self {
        let unsupported = || Support::unsupported("ML operation is not implemented");
        Self {
            exact_vector_search: unsupported(),
            approximate_vector_search: unsupported(),
            embedding: unsupported(),
            inference: unsupported(),
            max_vector_dimensions: 0,
            vector_metrics: Vec::new(),
        }
    }

    /// Declares exact vector support and its hard shape/metric boundary.
    #[must_use]
    pub fn with_vector_support(
        mut self,
        exact: Support,
        approximate: Support,
        max_dimensions: usize,
        metrics: impl IntoIterator<Item = Distance>,
    ) -> Self {
        self.exact_vector_search = exact;
        self.approximate_vector_search = approximate;
        self.max_vector_dimensions = max_dimensions;
        self.vector_metrics = metrics.into_iter().collect();
        self.vector_metrics
            .sort_unstable_by_key(|metric| match metric {
                Distance::Euclidean => 0,
                Distance::Cosine => 1,
                Distance::InnerProduct => 2,
            });
        self.vector_metrics.dedup();
        self
    }

    /// Maximum supported vector dimensions.
    #[must_use]
    pub const fn max_vector_dimensions(&self) -> usize {
        self.max_vector_dimensions
    }

    /// Exact supported metric set.
    #[must_use]
    pub fn vector_metrics(&self) -> &[Distance] {
        &self.vector_metrics
    }

    /// Evaluates one operation-specific capability instead of relying on a
    /// similarly named backend feature.
    #[must_use]
    pub fn support_for(&self, operation: MlOperation) -> Support {
        match operation {
            MlOperation::ExactVectorSearch { dimensions, metric } => {
                self.vector_support(dimensions, metric, false)
            }
            MlOperation::ApproximateVectorSearch { dimensions, metric } => {
                self.vector_support(dimensions, metric, true)
            }
            MlOperation::Embedding => self.embedding.clone(),
            MlOperation::Inference => self.inference.clone(),
        }
    }

    fn vector_support(&self, dimensions: usize, metric: Distance, approximate: bool) -> Support {
        let base = if approximate {
            &self.approximate_vector_search
        } else {
            &self.exact_vector_search
        };
        if !base.is_supported() {
            return base.clone();
        }
        if dimensions == 0 || dimensions > self.max_vector_dimensions {
            return Support::unsupported("vector dimension is outside the capability contract");
        }
        if !self.vector_metrics.contains(&metric) {
            return Support::unsupported("vector metric is outside the capability contract");
        }
        base.clone()
    }
}

impl Default for MlCapabilities {
    fn default() -> Self {
        Self::none()
    }
}

/// Semantic ML operation considered for placement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MlOperation {
    /// Exhaustive exact vector search.
    ExactVectorSearch {
        /// Query vector dimensions.
        dimensions: usize,
        /// Exact ranking metric.
        metric: Distance,
    },
    /// Approximate search; never substituted with exact search silently.
    ApproximateVectorSearch {
        /// Query vector dimensions.
        dimensions: usize,
        /// Ranking metric.
        metric: Distance,
    },
    /// Embedding generation.
    Embedding,
    /// General provider inference.
    Inference,
}

/// Result of ML-specific placement analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MlPlacement {
    /// Execute with the assigned engine/provider.
    Engine,
    /// Execute exact search with DOL's local exhaustive reference implementation.
    LocalExact,
}

/// Applies exact capability claims and the engine placement policy.
pub fn analyze_ml_placement(
    operation: MlOperation,
    capabilities: &MlCapabilities,
    policy: PlacementPolicy,
) -> Result<MlPlacement> {
    let support = capabilities.support_for(operation);
    if support.is_supported() {
        return Ok(MlPlacement::Engine);
    }
    match policy {
        PlacementPolicy::RemoteOnly => Err(MlError::new(
            "ML-PLACEMENT-001",
            support.reason().unwrap_or("ML operation is unsupported"),
        )),
        PlacementPolicy::Hybrid { transfer } => {
            transfer.validate().map_err(|error| {
                MlError::new(
                    "ML-PLACEMENT-002",
                    format!("invalid hybrid transfer budget: {}", error.message()),
                )
            })?;
            if matches!(operation, MlOperation::ExactVectorSearch { .. }) {
                Ok(MlPlacement::LocalExact)
            } else {
                Err(MlError::new(
                    "ML-PLACEMENT-003",
                    "no exact local residual exists for this ML operation",
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use dol_engine::{PlacementPolicy, Support, TransferBudget};

    use crate::vector::Distance;

    use super::{MlCapabilities, MlOperation, MlPlacement, analyze_ml_placement};

    #[test]
    fn placement_checks_dimensions_metrics_and_semantic_kind() {
        let capabilities = MlCapabilities::none().with_vector_support(
            Support::ExactNative,
            Support::unsupported("no ANN"),
            3,
            [Distance::Cosine],
        );
        assert_eq!(
            analyze_ml_placement(
                MlOperation::ExactVectorSearch {
                    dimensions: 3,
                    metric: Distance::Cosine,
                },
                &capabilities,
                PlacementPolicy::RemoteOnly,
            )
            .unwrap(),
            MlPlacement::Engine
        );
        let policy = PlacementPolicy::Hybrid {
            transfer: TransferBudget {
                max_rows: 10,
                max_bytes: 1_024,
                max_batches: Some(1),
            },
        };
        assert!(
            analyze_ml_placement(
                MlOperation::ApproximateVectorSearch {
                    dimensions: 3,
                    metric: Distance::Cosine,
                },
                &capabilities,
                policy,
            )
            .is_err()
        );
    }
}
