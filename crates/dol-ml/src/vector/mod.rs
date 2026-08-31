//! Dimension-aware vector values and distinct exact/approximate search semantics.

use core::cmp::Ordering;

use crate::limits::{SearchLimits, VectorLimits};
use crate::{MlError, Result};

/// Declared vector shape shared by a field, index, query, and result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VectorDefinition {
    dimensions: usize,
}

impl VectorDefinition {
    /// Creates a non-empty vector definition under the configured hard limit.
    pub fn new(dimensions: usize, limits: VectorLimits) -> Result<Self> {
        if dimensions == 0 {
            return Err(MlError::new(
                "ML-VECTOR-001",
                "vector dimensions must be greater than zero",
            ));
        }
        if dimensions > limits.max_dimensions() {
            return Err(MlError::new(
                "ML-VECTOR-002",
                "vector dimensions exceed the configured limit",
            ));
        }
        Ok(Self { dimensions })
    }

    /// Number of finite `f32` elements in every conforming value.
    #[must_use]
    pub const fn dimensions(self) -> usize {
        self.dimensions
    }
}

/// Validated dense finite floating-point vector.
#[derive(Debug, Clone, PartialEq)]
pub struct Vector {
    definition: VectorDefinition,
    values: Box<[f32]>,
}

impl Vector {
    /// Creates a vector with default limits.
    pub fn new(values: Vec<f32>) -> Result<Self> {
        Self::with_limits(values, VectorLimits::default())
    }

    /// Creates a vector with explicit construction limits.
    pub fn with_limits(values: Vec<f32>, limits: VectorLimits) -> Result<Self> {
        let definition = VectorDefinition::new(values.len(), limits)?;
        if values.iter().any(|value| !value.is_finite()) {
            return Err(MlError::new(
                "ML-VECTOR-003",
                "vector elements must be finite",
            ));
        }
        Ok(Self {
            definition,
            values: values.into_boxed_slice(),
        })
    }

    /// Declared vector shape.
    #[must_use]
    pub const fn definition(&self) -> VectorDefinition {
        self.definition
    }

    /// Returns the vector dimension.
    #[must_use]
    pub const fn dimensions(&self) -> usize {
        self.definition.dimensions
    }

    /// Borrows vector elements.
    #[must_use]
    pub fn as_slice(&self) -> &[f32] {
        &self.values
    }

    /// Computes the exact metric value, rejecting mismatched dimensions and
    /// undefined cosine distance for a zero vector.
    pub fn metric_value(&self, other: &Self, metric: Distance) -> Result<f64> {
        require_same_definition(self, other)?;
        match metric {
            Distance::Euclidean => Ok(self
                .values
                .iter()
                .zip(other.values.iter())
                .map(|(left, right)| {
                    let difference = f64::from(*left) - f64::from(*right);
                    difference * difference
                })
                .sum::<f64>()
                .sqrt()),
            Distance::Cosine => {
                let (dot, left_norm, right_norm) = dot_and_norms(self, other);
                if left_norm == 0.0 || right_norm == 0.0 {
                    return Err(MlError::new(
                        "ML-VECTOR-004",
                        "cosine distance is undefined for a zero vector",
                    ));
                }
                let similarity = (dot / (left_norm.sqrt() * right_norm.sqrt())).clamp(-1.0, 1.0);
                Ok(1.0 - similarity)
            }
            Distance::InnerProduct => Ok(dot_and_norms(self, other).0),
        }
    }
}

/// Compile-time dimension-checked vector wrapper.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StaticVector<const DIMENSIONS: usize> {
    values: [f32; DIMENSIONS],
}

impl<const DIMENSIONS: usize> StaticVector<DIMENSIONS> {
    /// Validates finite values and the compile-time dimension.
    pub fn new(values: [f32; DIMENSIONS]) -> Result<Self> {
        VectorDefinition::new(DIMENSIONS, VectorLimits::default())?;
        if values.iter().any(|value| !value.is_finite()) {
            return Err(MlError::new(
                "ML-VECTOR-003",
                "vector elements must be finite",
            ));
        }
        Ok(Self { values })
    }

    /// Borrows the statically sized elements.
    #[must_use]
    pub const fn as_array(&self) -> &[f32; DIMENSIONS] {
        &self.values
    }

    /// Converts to the dynamic semantic boundary.
    pub fn to_vector(self) -> Result<Vector> {
        Vector::new(self.values.to_vec())
    }
}

/// Vector distance/similarity metric.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Distance {
    /// Euclidean/L2 distance; smaller values are better.
    Euclidean,
    /// One minus cosine similarity; smaller values are better.
    Cosine,
    /// Raw inner-product score; larger values are better.
    InnerProduct,
}

impl Distance {
    /// Whether ranking minimizes a distance or maximizes a score.
    #[must_use]
    pub const fn preference(self) -> MetricPreference {
        match self {
            Self::Euclidean | Self::Cosine => MetricPreference::Minimize,
            Self::InnerProduct => MetricPreference::Maximize,
        }
    }
}

/// Ranking direction attached to a metric value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MetricPreference {
    /// Lower metric values rank first.
    Minimize,
    /// Higher metric values rank first.
    Maximize,
}

/// Stable caller-owned candidate identity used for deterministic tie breaking.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VectorId(String);

impl VectorId {
    /// Creates a bounded non-empty identity.
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if value.is_empty() || value.len() > 1_024 {
            return Err(MlError::new(
                "ML-VECTOR-005",
                "vector identity must contain 1..=1024 UTF-8 bytes",
            ));
        }
        Ok(Self(value))
    }

    /// Stable identity text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One vector eligible for exact local search.
#[derive(Debug, Clone, PartialEq)]
pub struct VectorCandidate {
    id: VectorId,
    vector: Vector,
}

impl VectorCandidate {
    /// Creates a candidate.
    #[must_use]
    pub const fn new(id: VectorId, vector: Vector) -> Self {
        Self { id, vector }
    }

    /// Candidate identity.
    #[must_use]
    pub const fn id(&self) -> &VectorId {
        &self.id
    }

    /// Candidate vector.
    #[must_use]
    pub const fn vector(&self) -> &Vector {
        &self.vector
    }
}

/// Semantically exact nearest-neighbor request.
#[derive(Debug, Clone, PartialEq)]
pub struct ExactSearchRequest {
    query: Vector,
    metric: Distance,
    limit: usize,
}

impl ExactSearchRequest {
    /// Creates an exact request under configured result limits.
    pub fn new(
        query: Vector,
        metric: Distance,
        limit: usize,
        limits: SearchLimits,
    ) -> Result<Self> {
        validate_result_limit(limit, limits)?;
        Ok(Self {
            query,
            metric,
            limit,
        })
    }

    /// Query vector.
    #[must_use]
    pub const fn query(&self) -> &Vector {
        &self.query
    }

    /// Exact ranking metric.
    #[must_use]
    pub const fn metric(&self) -> Distance {
        self.metric
    }

    /// Maximum returned matches.
    #[must_use]
    pub const fn limit(&self) -> usize {
        self.limit
    }
}

/// Explicit quality/work target for an approximate search.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ApproximationTarget {
    minimum_recall_basis_points: u16,
    probe_budget: usize,
}

impl ApproximationTarget {
    /// Creates a target where recall is in 1..=10,000 basis points.
    pub fn new(minimum_recall_basis_points: u16, probe_budget: usize) -> Result<Self> {
        if !(1..=10_000).contains(&minimum_recall_basis_points) || probe_budget == 0 {
            return Err(MlError::new(
                "ML-SEARCH-002",
                "approximation target requires non-zero probes and recall in 1..=10000 basis points",
            ));
        }
        Ok(Self {
            minimum_recall_basis_points,
            probe_budget,
        })
    }

    /// Requested lower-bound recall in basis points.
    #[must_use]
    pub const fn minimum_recall_basis_points(self) -> u16 {
        self.minimum_recall_basis_points
    }

    /// Maximum backend candidates/probes requested.
    #[must_use]
    pub const fn probe_budget(self) -> usize {
        self.probe_budget
    }
}

/// Approximate nearest-neighbor request, deliberately not interchangeable with
/// [`ExactSearchRequest`].
#[derive(Debug, Clone, PartialEq)]
pub struct ApproximateSearchRequest {
    query: Vector,
    metric: Distance,
    limit: usize,
    target: ApproximationTarget,
}

impl ApproximateSearchRequest {
    /// Creates a bounded approximate request.
    pub fn new(
        query: Vector,
        metric: Distance,
        limit: usize,
        target: ApproximationTarget,
        limits: SearchLimits,
    ) -> Result<Self> {
        validate_result_limit(limit, limits)?;
        if target.probe_budget > limits.max_candidates() {
            return Err(MlError::new(
                "ML-SEARCH-003",
                "approximate probe budget exceeds the configured candidate limit",
            ));
        }
        Ok(Self {
            query,
            metric,
            limit,
            target,
        })
    }

    /// Query vector.
    #[must_use]
    pub const fn query(&self) -> &Vector {
        &self.query
    }

    /// Ranking metric.
    #[must_use]
    pub const fn metric(&self) -> Distance {
        self.metric
    }

    /// Maximum returned matches.
    #[must_use]
    pub const fn limit(&self) -> usize {
        self.limit
    }

    /// Explicit approximation quality/work target.
    #[must_use]
    pub const fn target(&self) -> ApproximationTarget {
        self.target
    }
}

/// Search request retaining the exact-vs-approximate semantic distinction.
#[derive(Debug, Clone, PartialEq)]
pub enum VectorSearchRequest {
    /// Exhaustive exact ranking.
    Exact(ExactSearchRequest),
    /// Backend-specific approximate ranking under an explicit target.
    Approximate(ApproximateSearchRequest),
}

/// One deterministically ranked search result.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchMatch {
    id: VectorId,
    metric_value: f64,
}

impl SearchMatch {
    /// Candidate identity.
    #[must_use]
    pub const fn id(&self) -> &VectorId {
        &self.id
    }

    /// Distance or score according to the request metric.
    #[must_use]
    pub const fn metric_value(&self) -> f64 {
        self.metric_value
    }
}

/// Executes exhaustive deterministic local search. Approximate requests cannot
/// call this function by construction.
pub fn exact_search(
    request: &ExactSearchRequest,
    candidates: impl IntoIterator<Item = VectorCandidate>,
    limits: SearchLimits,
) -> Result<Vec<SearchMatch>> {
    validate_result_limit(request.limit, limits)?;
    let mut matches = Vec::new();
    for candidate in candidates {
        if matches.len() >= limits.max_candidates() {
            return Err(MlError::new(
                "ML-SEARCH-004",
                "exact search candidate count exceeds the configured limit",
            ));
        }
        let metric_value = request
            .query
            .metric_value(&candidate.vector, request.metric)?;
        matches.push(SearchMatch {
            id: candidate.id,
            metric_value,
        });
    }
    matches.sort_by(|left, right| rank_cmp(left, right, request.metric.preference()));
    matches.truncate(request.limit);
    Ok(matches)
}

fn rank_cmp(left: &SearchMatch, right: &SearchMatch, preference: MetricPreference) -> Ordering {
    let metric = match preference {
        MetricPreference::Minimize => left.metric_value.total_cmp(&right.metric_value),
        MetricPreference::Maximize => right.metric_value.total_cmp(&left.metric_value),
    };
    metric.then_with(|| left.id.cmp(&right.id))
}

fn validate_result_limit(limit: usize, limits: SearchLimits) -> Result<()> {
    if limit == 0 || limit > limits.max_results() {
        return Err(MlError::new(
            "ML-SEARCH-001",
            "search result limit must be non-zero and within the configured maximum",
        ));
    }
    Ok(())
}

fn require_same_definition(left: &Vector, right: &Vector) -> Result<()> {
    if left.definition == right.definition {
        Ok(())
    } else {
        Err(MlError::new(
            "ML-VECTOR-006",
            "vector dimensions do not match",
        ))
    }
}

fn dot_and_norms(left: &Vector, right: &Vector) -> (f64, f64, f64) {
    left.values.iter().zip(right.values.iter()).fold(
        (0.0, 0.0, 0.0),
        |(dot, left_norm, right_norm), (left, right)| {
            let left = f64::from(*left);
            let right = f64::from(*right);
            (
                dot + left * right,
                left_norm + left * left,
                right_norm + right * right,
            )
        },
    )
}

#[cfg(test)]
mod tests {
    use crate::limits::SearchLimits;

    use super::{
        Distance, ExactSearchRequest, StaticVector, Vector, VectorCandidate, VectorId, exact_search,
    };

    #[test]
    fn vector_validation_and_exact_search_are_deterministic() {
        assert!(Vector::new(vec![]).is_err());
        assert!(Vector::new(vec![f32::NAN]).is_err());

        let request = ExactSearchRequest::new(
            Vector::new(vec![0.0, 0.0]).unwrap(),
            Distance::Euclidean,
            2,
            SearchLimits::default(),
        )
        .unwrap();
        let candidates = vec![
            VectorCandidate::new(
                VectorId::new("b").unwrap(),
                Vector::new(vec![1.0, 0.0]).unwrap(),
            ),
            VectorCandidate::new(
                VectorId::new("a").unwrap(),
                Vector::new(vec![1.0, 0.0]).unwrap(),
            ),
            VectorCandidate::new(
                VectorId::new("far").unwrap(),
                Vector::new(vec![5.0, 0.0]).unwrap(),
            ),
        ];
        let result = exact_search(&request, candidates, SearchLimits::default()).unwrap();
        assert_eq!(result[0].id().as_str(), "a");
        assert_eq!(result[1].id().as_str(), "b");
    }

    #[test]
    fn static_dimensions_and_cosine_zero_are_checked() {
        assert!(StaticVector::<0>::new([]).is_err());
        let zero = StaticVector::new([0.0_f32, 0.0])
            .unwrap()
            .to_vector()
            .unwrap();
        let value = StaticVector::new([1.0_f32, 0.0])
            .unwrap()
            .to_vector()
            .unwrap();
        assert!(zero.metric_value(&value, Distance::Cosine).is_err());
    }
}
