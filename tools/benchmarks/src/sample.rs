//! Ordered raw performance observations.

use std::error::Error;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::scenario::{Lifecycle, ScenarioId, WorkUnitKind};
use crate::statistics::TukeyClass;

/// Revision side associated with one observation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RevisionSide {
    /// Reference revision.
    Baseline,
    /// Revision under evaluation.
    Candidate,
}

impl RevisionSide {
    /// Stable CSV spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Baseline => "baseline",
            Self::Candidate => "candidate",
        }
    }
}

/// Execution order of both sides in a complete temporal pair.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PairOrder {
    /// Run the reference revision, then the candidate.
    BaselineFirst,
    /// Run the candidate revision, then the reference.
    CandidateFirst,
}

impl PairOrder {
    /// Reverses pair order for a deterministic retry.
    #[must_use]
    pub const fn reversed(self) -> Self {
        match self {
            Self::BaselineFirst => Self::CandidateFirst,
            Self::CandidateFirst => Self::BaselineFirst,
        }
    }

    /// Stable CSV spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BaselineFirst => "baseline-first",
            Self::CandidateFirst => "candidate-first",
        }
    }
}

/// One raw, never-aggregated timing observation.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RawSample {
    /// Temporal pair number.
    pub pair: u32,
    /// Complete-pair attempt; zero is original and one is the sole retry.
    pub attempt: u32,
    /// Revision side.
    pub side: RevisionSide,
    /// Order in which this pair's sides were executed.
    pub pair_order: PairOrder,
    /// Global insertion order in the artifact.
    pub order: u64,
    /// Stable scenario identifier.
    pub scenario_id: ScenarioId,
    /// Independent lifecycle dimension.
    pub lifecycle: Lifecycle,
    /// Zero-based retained sample number within this side/scenario/attempt.
    pub sample: u32,
    /// Operation iterations in the timed region.
    pub iterations: u64,
    /// Timed-region duration.
    pub elapsed_ns: u64,
    /// Cost per operation.
    pub ns_per_op: f64,
    /// Normalized work units per second.
    pub throughput: f64,
    /// Work units completed by one operation.
    pub work_units: u64,
    /// Meaning of `work_units`.
    pub work_unit_kind: WorkUnitKind,
    /// Tukey classification computed over its complete side sample set.
    pub tukey_class: TukeyClass,
    /// Whether this observation contributes to the final paired estimate.
    pub valid_for_comparison: bool,
    /// Explicit reason for preserving but rejecting this observation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rejection_reason: Option<String>,
}

/// Invalid raw-sample structure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SampleError(String);

impl SampleError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for SampleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for SampleError {}

impl RawSample {
    /// Constructs a valid raw observation and derives all floating-point fields.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        pair: u32,
        attempt: u32,
        side: RevisionSide,
        pair_order: PairOrder,
        order: u64,
        scenario_id: ScenarioId,
        lifecycle: Lifecycle,
        sample: u32,
        iterations: u64,
        elapsed_ns: u64,
        work_units: u64,
        work_unit_kind: WorkUnitKind,
    ) -> Result<Self, SampleError> {
        if iterations == 0 {
            return Err(SampleError::new("sample iterations must be non-zero"));
        }
        if elapsed_ns == 0 {
            return Err(SampleError::new("sample elapsed_ns must be non-zero"));
        }
        if work_units == 0 {
            return Err(SampleError::new("sample work_units must be non-zero"));
        }
        let ns_per_op = elapsed_ns as f64 / iterations as f64;
        let throughput =
            work_units as f64 * iterations as f64 * 1_000_000_000.0 / elapsed_ns as f64;
        let observation = Self {
            pair,
            attempt,
            side,
            pair_order,
            order,
            scenario_id,
            lifecycle,
            sample,
            iterations,
            elapsed_ns,
            ns_per_op,
            throughput,
            work_units,
            work_unit_kind,
            tukey_class: TukeyClass::Inlier,
            valid_for_comparison: true,
            rejection_reason: None,
        };
        observation.validate()?;
        Ok(observation)
    }

    /// Validates invariants after deserialization.
    pub fn validate(&self) -> Result<(), SampleError> {
        if self.iterations == 0 || self.elapsed_ns == 0 || self.work_units == 0 {
            return Err(SampleError::new(format!(
                "sample {} has a zero iterations, elapsed_ns, or work_units field",
                self.order
            )));
        }
        if !self.ns_per_op.is_finite() || self.ns_per_op <= 0.0 {
            return Err(SampleError::new(format!(
                "sample {} has invalid ns_per_op",
                self.order
            )));
        }
        if !self.throughput.is_finite() || self.throughput <= 0.0 {
            return Err(SampleError::new(format!(
                "sample {} has invalid throughput",
                self.order
            )));
        }
        let expected_cost = self.elapsed_ns as f64 / self.iterations as f64;
        let expected_throughput = self.work_units as f64 * self.iterations as f64 * 1_000_000_000.0
            / self.elapsed_ns as f64;
        if !approximately_equal(self.ns_per_op, expected_cost)
            || !approximately_equal(self.throughput, expected_throughput)
        {
            return Err(SampleError::new(format!(
                "sample {} has inconsistent derived timing fields",
                self.order
            )));
        }
        match (self.valid_for_comparison, &self.rejection_reason) {
            (true, Some(reason)) if !reason.is_empty() => Err(SampleError::new(format!(
                "valid sample {} unexpectedly has a rejection reason",
                self.order
            ))),
            (false, None) => Err(SampleError::new(format!(
                "rejected sample {} is missing a rejection reason",
                self.order
            ))),
            (_, Some(reason)) if reason.trim().is_empty() => Err(SampleError::new(format!(
                "sample {} has an empty rejection reason",
                self.order
            ))),
            _ => Ok(()),
        }
    }

    /// Preserves the observation while excluding it from comparison.
    pub fn reject(&mut self, reason: impl Into<String>) -> Result<(), SampleError> {
        let reason = reason.into();
        if reason.trim().is_empty() {
            return Err(SampleError::new("rejection reason must not be empty"));
        }
        self.valid_for_comparison = false;
        self.rejection_reason = Some(reason);
        Ok(())
    }
}

/// Insertion-ordered sample collection. Statistical code sorts copies only.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OrderedRawSamples {
    samples: Vec<RawSample>,
}

impl OrderedRawSamples {
    /// Creates an empty collection.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            samples: Vec::new(),
        }
    }

    /// Validates existing insertion order and sample structure.
    pub fn from_vec(samples: Vec<RawSample>) -> Result<Self, SampleError> {
        let collection = Self { samples };
        collection.validate()?;
        Ok(collection)
    }

    /// Appends an already ordered observation.
    pub fn push(&mut self, sample: RawSample) -> Result<(), SampleError> {
        sample.validate()?;
        if let Some(previous) = self.samples.last()
            && sample.order <= previous.order
        {
            return Err(SampleError::new(format!(
                "sample order {} does not follow {}",
                sample.order, previous.order
            )));
        }
        self.samples.push(sample);
        Ok(())
    }

    /// Appends observations from an independently produced artifact and assigns
    /// fresh global order values while preserving their local insertion order.
    pub fn append_resequenced(
        &mut self,
        samples: impl IntoIterator<Item = RawSample>,
    ) -> Result<(), SampleError> {
        let mut next = self
            .samples
            .last()
            .map_or(0, |sample| sample.order.saturating_add(1));
        for mut sample in samples {
            sample.order = next;
            self.push(sample)?;
            next = next
                .checked_add(1)
                .ok_or_else(|| SampleError::new("raw sample order overflow"))?;
        }
        Ok(())
    }

    /// Borrows observations in original insertion order.
    #[must_use]
    pub fn as_slice(&self) -> &[RawSample] {
        &self.samples
    }

    /// Mutably borrows observations without changing their order.
    #[must_use]
    pub fn as_mut_slice(&mut self) -> &mut [RawSample] {
        &mut self.samples
    }

    /// Consumes the collection without sorting it.
    #[must_use]
    pub fn into_vec(self) -> Vec<RawSample> {
        self.samples
    }

    /// Number of retained observations, including rejected observations.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.samples.len()
    }

    /// Whether there are no observations.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Validates every observation and strictly increasing global order.
    pub fn validate(&self) -> Result<(), SampleError> {
        let mut previous = None;
        for sample in &self.samples {
            sample.validate()?;
            if previous.is_some_and(|order| sample.order <= order) {
                return Err(SampleError::new(format!(
                    "raw samples are not in insertion order at {}",
                    sample.order
                )));
            }
            previous = Some(sample.order);
        }
        Ok(())
    }
}

fn approximately_equal(left: f64, right: f64) -> bool {
    let scale = left.abs().max(right.abs()).max(1.0);
    (left - right).abs() <= scale * 1.0e-12
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(order: u64) -> RawSample {
        RawSample::new(
            0,
            0,
            RevisionSide::Baseline,
            PairOrder::BaselineFirst,
            order,
            ScenarioId::new("expr.eval.prepared").unwrap(),
            Lifecycle::Prepared,
            u32::try_from(order).unwrap(),
            10,
            250,
            1,
            WorkUnitKind::Evaluation,
        )
        .unwrap()
    }

    #[test]
    fn ordered_samples_reject_reordering() {
        let mut samples = OrderedRawSamples::new();
        samples.push(sample(4)).unwrap();
        assert!(samples.push(sample(3)).is_err());
        assert_eq!(samples.as_slice()[0].order, 4);
    }

    #[test]
    fn independently_ordered_batches_can_be_resequenced() {
        let mut samples = OrderedRawSamples::new();
        samples.append_resequenced([sample(9), sample(10)]).unwrap();
        samples.append_resequenced([sample(0), sample(1)]).unwrap();
        assert_eq!(
            samples
                .as_slice()
                .iter()
                .map(|sample| sample.order)
                .collect::<Vec<_>>(),
            vec![0, 1, 2, 3]
        );
    }

    #[test]
    fn rejected_observation_requires_and_retains_a_reason() {
        let mut observation = sample(0);
        observation.reject("noise-threshold").unwrap();
        observation.validate().unwrap();
        assert!(!observation.valid_for_comparison);
        assert_eq!(
            observation.rejection_reason.as_deref(),
            Some("noise-threshold")
        );
    }

    #[test]
    fn derived_cost_and_throughput_are_consistent() {
        let observation = sample(0);
        assert_eq!(observation.ns_per_op, 25.0);
        assert_eq!(observation.throughput, 40_000_000.0);
    }
}
