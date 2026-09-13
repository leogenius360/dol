//! Structural paired comparison, deterministic bootstrap, and finite verdicts.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::sample::{OrderedRawSamples, PairOrder, RawSample, RevisionSide};
use crate::scenario::{Lifecycle, ScenarioId};
use crate::statistics::{
    DescriptiveStatistics, StatisticsError, TukeyClass, describe, median, quantile,
};

/// Bootstrap draws frozen by `closure-v1`.
pub const BOOTSTRAP_RESAMPLES: usize = 100_000;
/// Fixed default seed, hex-encoded as `DOL-closure-v1` followed by zero padding.
pub const DEFAULT_BOOTSTRAP_SEED: BootstrapSeed = BootstrapSeed([
    0x44, 0x4f, 0x4c, 0x2d, 0x63, 0x6c, 0x6f, 0x73, 0x75, 0x72, 0x65, 0x2d, 0x76, 0x31, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
]);

/// Deterministic 256-bit bootstrap seed.
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub struct BootstrapSeed(pub [u8; 32]);

impl BootstrapSeed {
    /// Parses exactly 64 hexadecimal characters.
    pub fn from_hex(value: &str) -> Result<Self, ComparisonError> {
        if value.len() != 64 {
            return Err(ComparisonError::new(format!(
                "bootstrap seed must contain exactly 64 hexadecimal characters, found {}",
                value.len()
            )));
        }
        let mut bytes = [0_u8; 32];
        let (pairs, remainder) = value.as_bytes().as_chunks::<2>();
        debug_assert!(remainder.is_empty());
        for (index, pair) in pairs.iter().enumerate() {
            bytes[index] = (hex_nibble(pair[0])? << 4) | hex_nibble(pair[1])?;
        }
        Ok(Self(bytes))
    }

    /// Canonical lower-case fixed-width representation.
    #[must_use]
    pub fn to_hex(self) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut rendered = String::with_capacity(64);
        for byte in self.0 {
            rendered.push(char::from(HEX[usize::from(byte >> 4)]));
            rendered.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
        rendered
    }
}

impl fmt::Debug for BootstrapSeed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("BootstrapSeed")
            .field(&self.to_hex())
            .finish()
    }
}

impl fmt::Display for BootstrapSeed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_hex())
    }
}

impl Serialize for BootstrapSeed {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for BootstrapSeed {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::from_hex(&value).map_err(serde::de::Error::custom)
    }
}

/// Closed set of serialized comparison outcomes.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Verdict {
    /// At least 10% lower median cost with a CI wholly below zero.
    Improvement,
    /// No threshold-crossing signal.
    Neutral,
    /// At least 3% slower, without meeting the hard-regression rule.
    Warning,
    /// At least 5% slower and the CI lower bound is above 3%.
    HardRegression,
    /// Variability exceeded the frozen noise policy after retry handling.
    Noisy,
    /// Complete data exists, but cannot support a threshold conclusion.
    Inconclusive,
    /// Structural/provenance requirements were not met.
    InvalidComparison,
}

/// Frozen confidence interval bounds on the paired cost-ratio axis.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct ConfidenceInterval {
    /// Type-7 2.5th percentile of deterministic bootstrap medians.
    pub lower: f64,
    /// Type-7 97.5th percentile of deterministic bootstrap medians.
    pub upper: f64,
}

/// Paired cost-ratio estimate (`candidate / baseline - 1`).
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct BootstrapEstimate {
    /// One ratio for each structurally complete temporal pair.
    pub paired_ratios: Vec<f64>,
    /// Average-middle median of paired ratios.
    pub median_ratio: f64,
    /// Deterministic percentile interval.
    pub confidence_interval_95: ConfidenceInterval,
    /// Resampling count, always 100,000 for this contract.
    pub resamples: usize,
    /// Seed that produced this estimate.
    pub seed: BootstrapSeed,
}

/// Frozen verdict thresholds.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct DecisionRules {
    /// Median ratio at which an improvement may be retained.
    pub improvement_median: f64,
    /// Median ratio at which a warning begins.
    pub warning_median: f64,
    /// Median ratio component of a hard regression.
    pub hard_regression_median: f64,
    /// CI lower-bound component of a hard regression.
    pub hard_regression_ci_lower: f64,
}

impl Default for DecisionRules {
    fn default() -> Self {
        Self {
            improvement_median: -0.10,
            warning_median: 0.03,
            hard_regression_median: 0.05,
            hard_regression_ci_lower: 0.03,
        }
    }
}

/// Frozen per-side noise thresholds.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct NoiseRules {
    /// Maximum dimensionless normalized MAD.
    pub max_normalized_mad: f64,
    /// Maximum population coefficient of variation.
    pub max_coefficient_of_variation: f64,
    /// Maximum fraction of observations beyond inner Tukey fences.
    pub max_tukey_outlier_fraction: f64,
}

impl Default for NoiseRules {
    fn default() -> Self {
        Self {
            max_normalized_mad: 0.05,
            max_coefficient_of_variation: 0.10,
            max_tukey_outlier_fraction: 0.20,
        }
    }
}

/// Configuration for complete-pair analysis.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct ComparisonConfig {
    /// Bootstrap seed shared across scenario estimates.
    pub bootstrap_seed: BootstrapSeed,
    /// Expected minimum number of valid complete pairs.
    pub minimum_pairs: usize,
    /// Number of permitted complete-pair retries; frozen to one by default.
    pub max_complete_pair_retries: u32,
    /// Verdict rules.
    pub decision_rules: DecisionRules,
    /// Noise rules.
    pub noise_rules: NoiseRules,
}

impl Default for ComparisonConfig {
    fn default() -> Self {
        Self {
            bootstrap_seed: DEFAULT_BOOTSTRAP_SEED,
            minimum_pairs: 2,
            max_complete_pair_retries: 1,
            decision_rules: DecisionRules::default(),
            noise_rules: NoiseRules::default(),
        }
    }
}

impl ComparisonConfig {
    /// Validates configuration without relaxing `closure-v1` invariants.
    pub fn validate(self) -> Result<(), ComparisonError> {
        if self.minimum_pairs == 0 {
            return Err(ComparisonError::new("minimum_pairs must be non-zero"));
        }
        if self.max_complete_pair_retries != 1 {
            return Err(ComparisonError::new(
                "closure-v1 permits exactly one complete-pair retry",
            ));
        }
        let finite = [
            self.decision_rules.improvement_median,
            self.decision_rules.warning_median,
            self.decision_rules.hard_regression_median,
            self.decision_rules.hard_regression_ci_lower,
            self.noise_rules.max_normalized_mad,
            self.noise_rules.max_coefficient_of_variation,
            self.noise_rules.max_tukey_outlier_fraction,
        ]
        .into_iter()
        .all(f64::is_finite);
        if !finite {
            return Err(ComparisonError::new(
                "comparison thresholds must all be finite",
            ));
        }
        if self.noise_rules.max_normalized_mad < 0.0
            || self.noise_rules.max_coefficient_of_variation < 0.0
            || !(0.0..=1.0).contains(&self.noise_rules.max_tukey_outlier_fraction)
        {
            return Err(ComparisonError::new("invalid noise threshold"));
        }
        Ok(())
    }
}

/// Retry state represented in final evidence.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RetryStatus {
    /// Original observations passed the noise gate.
    NotNeeded,
    /// A complete replacement pair was retained.
    Retried,
    /// A complete replacement pair must still be run.
    Required,
    /// The single permitted replacement pair was also noisy.
    Exhausted,
}

/// One side/scenario summary in the decision input.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ScenarioSummary {
    /// Stable scenario identifier.
    pub scenario_id: ScenarioId,
    /// Independent lifecycle dimension.
    pub lifecycle: Lifecycle,
    /// Revision side.
    pub side: RevisionSide,
    /// Statistics over comparison-valid observations only.
    pub statistics: DescriptiveStatistics,
}

/// Final decision for one scenario/lifecycle compound key.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PairedComparison {
    /// Stable scenario identifier.
    pub scenario_id: ScenarioId,
    /// Independent lifecycle dimension.
    pub lifecycle: Lifecycle,
    /// Structurally complete pairs contributing to the estimate.
    pub pair_count: usize,
    /// Paired deterministic estimate, absent only for non-estimable outcomes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimate: Option<BootstrapEstimate>,
    /// Finite classification.
    pub classification: Verdict,
    /// Deterministic human-readable rationale.
    pub reason: String,
    /// Aggregate retry state for contributing/affected pairs.
    pub retry_status: RetryStatus,
}

/// Request to replace both sides of one temporal pair.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RetryRequest {
    /// Pair to rerun in full.
    pub pair: u32,
    /// Attempt whose complete observations were rejected.
    pub rejected_attempt: u32,
    /// Replacement attempt number.
    pub retry_attempt: u32,
    /// Reversed ordering for the replacement pair.
    pub pair_order: PairOrder,
    /// First scenario that crossed a frozen noise threshold.
    pub trigger_scenario_id: ScenarioId,
    /// Lifecycle of the triggering scenario.
    pub trigger_lifecycle: Lifecycle,
}

/// Result of analysis. Retry requests are empty only when evidence is final.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct ComparisonAnalysis {
    /// Single-side descriptive summaries.
    pub summaries: Vec<ScenarioSummary>,
    /// Scenario-level finite verdicts.
    pub comparisons: Vec<PairedComparison>,
    /// Whole-pair work that must be rerun before a final verdict.
    pub retry_requests: Vec<RetryRequest>,
}

/// Invalid paired data or bootstrap input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComparisonError(String);

impl ComparisonError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for ComparisonError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for ComparisonError {}

impl From<StatisticsError> for ComparisonError {
    fn from(error: StatisticsError) -> Self {
        Self::new(error.to_string())
    }
}

/// Executes the immutable 100,000-resample paired bootstrap.
pub fn paired_bootstrap(
    baseline_costs: &[f64],
    candidate_costs: &[f64],
    seed: BootstrapSeed,
) -> Result<BootstrapEstimate, ComparisonError> {
    if baseline_costs.is_empty() {
        return Err(ComparisonError::new(
            "paired bootstrap requires at least one complete pair",
        ));
    }
    if baseline_costs.len() != candidate_costs.len() {
        return Err(ComparisonError::new(format!(
            "paired bootstrap length mismatch: baseline={}, candidate={}",
            baseline_costs.len(),
            candidate_costs.len()
        )));
    }
    let paired_ratios = baseline_costs
        .iter()
        .zip(candidate_costs)
        .enumerate()
        .map(|(index, (baseline, candidate))| {
            if !baseline.is_finite()
                || !candidate.is_finite()
                || *baseline <= 0.0
                || *candidate <= 0.0
            {
                return Err(ComparisonError::new(format!(
                    "pair {index} has non-finite or non-positive cost"
                )));
            }
            let ratio = candidate / baseline - 1.0;
            if ratio.is_finite() {
                Ok(ratio)
            } else {
                Err(ComparisonError::new(format!(
                    "pair {index} produced a non-finite ratio"
                )))
            }
        })
        .collect::<Result<Vec<_>, _>>()?;

    let median_ratio = median(&paired_ratios)?;
    let mut generator = Xoshiro256StarStar::from_seed(seed);
    let mut bootstrap_medians = Vec::with_capacity(BOOTSTRAP_RESAMPLES);
    let mut resample = vec![0.0; paired_ratios.len()];
    for _ in 0..BOOTSTRAP_RESAMPLES {
        for value in &mut resample {
            *value = paired_ratios[generator.index(paired_ratios.len())];
        }
        bootstrap_medians.push(median(&resample)?);
    }
    let lower = quantile(&bootstrap_medians, 0.025)?;
    let upper = quantile(&bootstrap_medians, 0.975)?;
    if !median_ratio.is_finite() || !lower.is_finite() || !upper.is_finite() {
        return Err(ComparisonError::new(
            "bootstrap estimate produced a non-finite value",
        ));
    }
    Ok(BootstrapEstimate {
        paired_ratios,
        median_ratio,
        confidence_interval_95: ConfidenceInterval { lower, upper },
        resamples: BOOTSTRAP_RESAMPLES,
        seed,
    })
}

/// Applies the finite decision table to one paired estimate.
#[must_use]
pub fn classify_estimate(estimate: &BootstrapEstimate, rules: DecisionRules) -> (Verdict, String) {
    let median = estimate.median_ratio;
    let interval = estimate.confidence_interval_95;
    if median >= rules.hard_regression_median && interval.lower > rules.hard_regression_ci_lower {
        (
            Verdict::HardRegression,
            format!(
                "median cost rose {:.2}% and CI lower bound {:.2}% exceeded {:.2}%",
                median * 100.0,
                interval.lower * 100.0,
                rules.hard_regression_ci_lower * 100.0
            ),
        )
    } else if median <= rules.improvement_median && interval.upper < 0.0 {
        (
            Verdict::Improvement,
            format!(
                "median cost changed {:.2}% and CI upper bound {:.2}% remained below zero",
                median * 100.0,
                interval.upper * 100.0
            ),
        )
    } else if median >= rules.warning_median {
        (
            Verdict::Warning,
            format!(
                "median cost rose {:.2}%, meeting the {:.2}% warning threshold",
                median * 100.0,
                rules.warning_median * 100.0
            ),
        )
    } else if (median <= rules.improvement_median && interval.upper >= 0.0)
        || interval.upper - interval.lower > 0.10
    {
        (
            Verdict::Inconclusive,
            "confidence interval is too broad for a threshold conclusion".into(),
        )
    } else {
        (
            Verdict::Neutral,
            "paired cost did not cross a closure-v1 decision threshold".into(),
        )
    }
}

/// Structurally matches pair/attempt/scenario keys, annotates raw samples, rejects
/// an entire pair when either side is noisy, and builds final comparisons.
pub fn analyze_complete_pairs(
    samples: &mut OrderedRawSamples,
    config: &ComparisonConfig,
) -> Result<ComparisonAnalysis, ComparisonError> {
    config.validate()?;
    samples
        .validate()
        .map_err(|error| ComparisonError::new(error.to_string()))?;
    if samples.is_empty() {
        return Ok(ComparisonAnalysis::default());
    }

    let selected_attempts = latest_attempts_by_pair(samples.as_slice());
    let mut group_indices = group_selected_samples(samples.as_slice(), &selected_attempts);
    let mut pair_noise = BTreeMap::<u32, NoiseTrigger>::new();
    let mut invalid_pairs = BTreeSet::<u32>::new();

    for (key, group) in &mut group_indices {
        if group.baseline.is_empty()
            || group.candidate.is_empty()
            || group.baseline.len() != group.candidate.len()
            || group.pair_orders.len() != 1
        {
            invalid_pairs.insert(key.pair);
            continue;
        }

        let baseline_values = costs_at(samples.as_slice(), &group.baseline);
        let candidate_values = costs_at(samples.as_slice(), &group.candidate);
        let (baseline_stats, baseline_classes) = describe(&baseline_values)?;
        let (candidate_stats, candidate_classes) = describe(&candidate_values)?;
        assign_tukey(samples.as_mut_slice(), &group.baseline, &baseline_classes);
        assign_tukey(samples.as_mut_slice(), &group.candidate, &candidate_classes);

        if noisy(&baseline_stats, config.noise_rules) || noisy(&candidate_stats, config.noise_rules)
        {
            pair_noise.entry(key.pair).or_insert_with(|| NoiseTrigger {
                scenario_id: key.scenario_id.clone(),
                lifecycle: key.lifecycle,
                pair_order: *group
                    .pair_orders
                    .first()
                    .expect("one pair order was validated"),
            });
        }
    }

    for pair in &invalid_pairs {
        reject_pair_attempt(samples, *pair, selected_attempts[pair], "incomplete-pair")?;
    }

    let mut retry_requests = Vec::new();
    let mut exhausted_pairs = BTreeSet::new();
    for (pair, trigger) in &pair_noise {
        let attempt = selected_attempts[pair];
        reject_pair_attempt(samples, *pair, attempt, "noise-threshold")?;
        if attempt < config.max_complete_pair_retries {
            retry_requests.push(RetryRequest {
                pair: *pair,
                rejected_attempt: attempt,
                retry_attempt: attempt + 1,
                pair_order: trigger.pair_order.reversed(),
                trigger_scenario_id: trigger.scenario_id.clone(),
                trigger_lifecycle: trigger.lifecycle,
            });
        } else {
            exhausted_pairs.insert(*pair);
        }
    }

    let summaries = summarize_valid_samples(samples.as_slice())?;
    let comparisons = build_comparisons(
        samples.as_slice(),
        config,
        &retry_requests,
        &exhausted_pairs,
        &invalid_pairs,
    )?;
    Ok(ComparisonAnalysis {
        summaries,
        comparisons,
        retry_requests,
    })
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct AttemptScenarioKey {
    pair: u32,
    attempt: u32,
    scenario_id: ScenarioId,
    lifecycle: Lifecycle,
}

#[derive(Default)]
struct SampleGroup {
    baseline: Vec<usize>,
    candidate: Vec<usize>,
    pair_orders: BTreeSet<PairOrder>,
}

struct NoiseTrigger {
    scenario_id: ScenarioId,
    lifecycle: Lifecycle,
    pair_order: PairOrder,
}

fn latest_attempts_by_pair(samples: &[RawSample]) -> BTreeMap<u32, u32> {
    let mut latest = BTreeMap::new();
    for sample in samples {
        latest
            .entry(sample.pair)
            .and_modify(|attempt: &mut u32| *attempt = (*attempt).max(sample.attempt))
            .or_insert(sample.attempt);
    }
    latest
}

fn group_selected_samples(
    samples: &[RawSample],
    selected_attempts: &BTreeMap<u32, u32>,
) -> BTreeMap<AttemptScenarioKey, SampleGroup> {
    let mut groups = BTreeMap::new();
    for (index, sample) in samples.iter().enumerate() {
        if selected_attempts.get(&sample.pair) != Some(&sample.attempt) {
            continue;
        }
        let key = AttemptScenarioKey {
            pair: sample.pair,
            attempt: sample.attempt,
            scenario_id: sample.scenario_id.clone(),
            lifecycle: sample.lifecycle,
        };
        let group = groups.entry(key).or_insert_with(SampleGroup::default);
        match sample.side {
            RevisionSide::Baseline => group.baseline.push(index),
            RevisionSide::Candidate => group.candidate.push(index),
        }
        group.pair_orders.insert(sample.pair_order);
    }
    groups
}

fn costs_at(samples: &[RawSample], indices: &[usize]) -> Vec<f64> {
    indices
        .iter()
        .map(|index| samples[*index].ns_per_op)
        .collect()
}

fn assign_tukey(samples: &mut [RawSample], indices: &[usize], classes: &[TukeyClass]) {
    debug_assert_eq!(indices.len(), classes.len());
    for (index, class) in indices.iter().zip(classes) {
        samples[*index].tukey_class = *class;
    }
}

fn noisy(statistics: &DescriptiveStatistics, rules: NoiseRules) -> bool {
    let outlier_fraction = statistics.outliers.total() as f64 / statistics.sample_count as f64;
    statistics.normalized_mad > rules.max_normalized_mad
        || statistics.coefficient_of_variation > rules.max_coefficient_of_variation
        || outlier_fraction > rules.max_tukey_outlier_fraction
}

fn reject_pair_attempt(
    samples: &mut OrderedRawSamples,
    pair: u32,
    attempt: u32,
    reason: &str,
) -> Result<(), ComparisonError> {
    for sample in samples
        .as_mut_slice()
        .iter_mut()
        .filter(|sample| sample.pair == pair && sample.attempt == attempt)
    {
        sample
            .reject(reason)
            .map_err(|error| ComparisonError::new(error.to_string()))?;
    }
    Ok(())
}

/// Builds single-side summaries from observations currently valid for comparison.
pub fn summarize_valid_samples(
    samples: &[RawSample],
) -> Result<Vec<ScenarioSummary>, ComparisonError> {
    let mut grouped = BTreeMap::<(ScenarioId, Lifecycle, RevisionSide), Vec<f64>>::new();
    for sample in samples.iter().filter(|sample| sample.valid_for_comparison) {
        grouped
            .entry((sample.scenario_id.clone(), sample.lifecycle, sample.side))
            .or_default()
            .push(sample.ns_per_op);
    }
    grouped
        .into_iter()
        .map(|((scenario_id, lifecycle, side), values)| {
            let (statistics, _) = describe(&values)?;
            Ok(ScenarioSummary {
                scenario_id,
                lifecycle,
                side,
                statistics,
            })
        })
        .collect()
}

#[derive(Default)]
struct PairCosts {
    baseline: Vec<f64>,
    candidate: Vec<f64>,
    attempts: BTreeSet<u32>,
}

fn build_comparisons(
    samples: &[RawSample],
    config: &ComparisonConfig,
    retry_requests: &[RetryRequest],
    exhausted_pairs: &BTreeSet<u32>,
    invalid_pairs: &BTreeSet<u32>,
) -> Result<Vec<PairedComparison>, ComparisonError> {
    let mut grouped = BTreeMap::<(ScenarioId, Lifecycle, u32), PairCosts>::new();
    let mut all_scenarios = BTreeSet::<(ScenarioId, Lifecycle)>::new();
    for sample in samples {
        all_scenarios.insert((sample.scenario_id.clone(), sample.lifecycle));
        if !sample.valid_for_comparison {
            continue;
        }
        let costs = grouped
            .entry((sample.scenario_id.clone(), sample.lifecycle, sample.pair))
            .or_default();
        match sample.side {
            RevisionSide::Baseline => costs.baseline.push(sample.ns_per_op),
            RevisionSide::Candidate => costs.candidate.push(sample.ns_per_op),
        }
        costs.attempts.insert(sample.attempt);
    }

    let pending_pairs = retry_requests
        .iter()
        .map(|request| request.pair)
        .collect::<BTreeSet<_>>();
    let mut output = Vec::with_capacity(all_scenarios.len());
    for (scenario_id, lifecycle) in all_scenarios {
        let relevant = grouped
            .iter()
            .filter(|((id, life, _), _)| id == &scenario_id && *life == lifecycle)
            .collect::<Vec<_>>();
        let mut baseline_costs = Vec::new();
        let mut candidate_costs = Vec::new();
        let mut retried = false;
        let mut structural_failure = false;
        for ((_, _, pair), costs) in relevant {
            if costs.baseline.is_empty() || costs.candidate.is_empty() || costs.attempts.len() != 1
            {
                structural_failure = true;
                continue;
            }
            baseline_costs.push((*pair, median(&costs.baseline)?));
            candidate_costs.push((*pair, median(&costs.candidate)?));
            retried |= costs.attempts.first().is_some_and(|attempt| *attempt > 0);
        }
        baseline_costs.sort_by_key(|(pair, _)| *pair);
        candidate_costs.sort_by_key(|(pair, _)| *pair);
        if baseline_costs
            .iter()
            .map(|(pair, _)| pair)
            .ne(candidate_costs.iter().map(|(pair, _)| pair))
        {
            structural_failure = true;
        }
        let baseline = baseline_costs
            .iter()
            .map(|(_, cost)| *cost)
            .collect::<Vec<_>>();
        let candidate = candidate_costs
            .iter()
            .map(|(_, cost)| *cost)
            .collect::<Vec<_>>();
        let affected_pending = !pending_pairs.is_empty();
        let affected_exhausted = !exhausted_pairs.is_empty();
        let affected_invalid = structural_failure || !invalid_pairs.is_empty();
        let retry_status = if affected_pending {
            RetryStatus::Required
        } else if affected_exhausted {
            RetryStatus::Exhausted
        } else if retried {
            RetryStatus::Retried
        } else {
            RetryStatus::NotNeeded
        };

        let comparison = if affected_pending {
            PairedComparison {
                scenario_id,
                lifecycle,
                pair_count: baseline.len(),
                estimate: None,
                classification: Verdict::Noisy,
                reason: "a noisy complete pair must be rerun on both sides".into(),
                retry_status,
            }
        } else if affected_exhausted {
            PairedComparison {
                scenario_id,
                lifecycle,
                pair_count: baseline.len(),
                estimate: None,
                classification: Verdict::Noisy,
                reason: "the single complete-pair noise retry was exhausted".into(),
                retry_status,
            }
        } else if affected_invalid {
            PairedComparison {
                scenario_id,
                lifecycle,
                pair_count: baseline.len(),
                estimate: None,
                classification: Verdict::InvalidComparison,
                reason: "one or more temporal pairs were structurally incomplete".into(),
                retry_status,
            }
        } else if baseline.len() < config.minimum_pairs {
            PairedComparison {
                scenario_id,
                lifecycle,
                pair_count: baseline.len(),
                estimate: None,
                classification: Verdict::Inconclusive,
                reason: format!(
                    "{} complete pairs were available; {} are required",
                    baseline.len(),
                    config.minimum_pairs
                ),
                retry_status,
            }
        } else {
            let estimate = paired_bootstrap(&baseline, &candidate, config.bootstrap_seed)?;
            let (classification, reason) = classify_estimate(&estimate, config.decision_rules);
            PairedComparison {
                scenario_id,
                lifecycle,
                pair_count: baseline.len(),
                estimate: Some(estimate),
                classification,
                reason,
                retry_status,
            }
        };
        output.push(comparison);
    }
    Ok(output)
}

fn hex_nibble(byte: u8) -> Result<u8, ComparisonError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(ComparisonError::new(format!(
            "invalid hexadecimal bootstrap seed character `{}`",
            char::from(byte)
        ))),
    }
}

struct Xoshiro256StarStar {
    state: [u64; 4],
}

impl Xoshiro256StarStar {
    fn from_seed(seed: BootstrapSeed) -> Self {
        let mut state = [0_u64; 4];
        let (chunks, remainder) = seed.0.as_chunks::<8>();
        debug_assert!(remainder.is_empty());
        for (slot, bytes) in state.iter_mut().zip(chunks) {
            *slot = u64::from_le_bytes(*bytes);
        }
        if state == [0; 4] {
            // xoshiro's all-zero state is absorbing; keep even that serialized seed valid.
            state[0] = 0x9e37_79b9_7f4a_7c15;
        }
        Self { state }
    }

    fn next_u64(&mut self) -> u64 {
        let result = self.state[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let temporary = self.state[1] << 17;
        self.state[2] ^= self.state[0];
        self.state[3] ^= self.state[1];
        self.state[1] ^= self.state[2];
        self.state[0] ^= self.state[3];
        self.state[2] ^= temporary;
        self.state[3] = self.state[3].rotate_left(45);
        result
    }

    fn index(&mut self, length: usize) -> usize {
        // Multiply-high mapping avoids modulo bias for practical pair counts.
        let random = u128::from(self.next_u64());
        let scaled = random * length as u128;
        usize::try_from(scaled >> 64).expect("scaled index is always below usize length")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample::RawSample;
    use crate::scenario::WorkUnitKind;

    const EXPECTED_SEED: &str = "444f4c2d636c6f737572652d7631000000000000000000000000000000000000";

    #[test]
    fn seed_serialization_is_fixed_width_hex_not_a_json_number() {
        assert_eq!(DEFAULT_BOOTSTRAP_SEED.to_hex(), EXPECTED_SEED);
        assert_eq!(
            serde_json::to_string(&DEFAULT_BOOTSTRAP_SEED).unwrap(),
            format!("\"{EXPECTED_SEED}\"")
        );
        assert_eq!(
            serde_json::from_str::<BootstrapSeed>(&format!("\"{EXPECTED_SEED}\"")).unwrap(),
            DEFAULT_BOOTSTRAP_SEED
        );
        assert!(BootstrapSeed::from_hex("01").is_err());
    }

    #[test]
    fn paired_bootstrap_is_deterministic_and_uses_one_ratio_per_pair() {
        let baseline = [100.0, 110.0, 90.0, 105.0];
        let candidate = [90.0, 99.0, 81.0, 94.5];
        let first = paired_bootstrap(&baseline, &candidate, DEFAULT_BOOTSTRAP_SEED).unwrap();
        let second = paired_bootstrap(&baseline, &candidate, DEFAULT_BOOTSTRAP_SEED).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.resamples, 100_000);
        assert_eq!(first.paired_ratios.len(), 4);
        assert!((first.median_ratio + 0.1).abs() < 1.0e-12);
        assert!(first.confidence_interval_95.upper < 0.0);
    }

    #[test]
    fn finite_verdict_table_matches_closure_thresholds() {
        let estimate = |median_ratio, lower, upper| BootstrapEstimate {
            paired_ratios: vec![median_ratio; 7],
            median_ratio,
            confidence_interval_95: ConfidenceInterval { lower, upper },
            resamples: BOOTSTRAP_RESAMPLES,
            seed: DEFAULT_BOOTSTRAP_SEED,
        };
        assert_eq!(
            classify_estimate(&estimate(0.05, 0.031, 0.08), DecisionRules::default()).0,
            Verdict::HardRegression
        );
        assert_eq!(
            classify_estimate(&estimate(0.049, 0.04, 0.06), DecisionRules::default()).0,
            Verdict::Warning
        );
        assert_eq!(
            classify_estimate(&estimate(-0.10, -0.14, -0.01), DecisionRules::default()).0,
            Verdict::Improvement
        );
        assert_eq!(
            classify_estimate(&estimate(0.0, -0.01, 0.01), DecisionRules::default()).0,
            Verdict::Neutral
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn raw(
        order: u64,
        pair: u32,
        attempt: u32,
        side: RevisionSide,
        pair_order: PairOrder,
        sample: u32,
        ns: u64,
    ) -> RawSample {
        RawSample::new(
            pair,
            attempt,
            side,
            pair_order,
            order,
            ScenarioId::new("expr.eval.prepared").unwrap(),
            Lifecycle::Prepared,
            sample,
            1,
            ns,
            1,
            WorkUnitKind::Evaluation,
        )
        .unwrap()
    }

    #[test]
    fn noisy_side_rejects_and_retries_both_sides_without_dropping_observations() {
        let mut raw_samples = Vec::new();
        let baseline = [100, 100, 100, 100, 300];
        let candidate = [90, 90, 90, 90, 90];
        let mut order = 0;
        for (side, values) in [
            (RevisionSide::Baseline, baseline.as_slice()),
            (RevisionSide::Candidate, candidate.as_slice()),
        ] {
            for (sample, value) in values.iter().enumerate() {
                raw_samples.push(raw(
                    order,
                    4,
                    0,
                    side,
                    PairOrder::BaselineFirst,
                    u32::try_from(sample).unwrap(),
                    *value,
                ));
                order += 1;
            }
        }
        let mut samples = OrderedRawSamples::from_vec(raw_samples).unwrap();
        let analysis = analyze_complete_pairs(&mut samples, &ComparisonConfig::default()).unwrap();
        assert_eq!(analysis.retry_requests.len(), 1);
        assert_eq!(analysis.retry_requests[0].pair, 4);
        assert_eq!(analysis.retry_requests[0].retry_attempt, 1);
        assert_eq!(
            analysis.retry_requests[0].pair_order,
            PairOrder::CandidateFirst
        );
        assert_eq!(samples.len(), 10);
        assert!(samples.as_slice().iter().all(|sample| {
            !sample.valid_for_comparison
                && sample.rejection_reason.as_deref() == Some("noise-threshold")
        }));
    }

    #[test]
    fn structural_pairing_does_not_zip_unrelated_pair_numbers() {
        let mut samples = OrderedRawSamples::new();
        samples
            .append_resequenced([
                raw(
                    0,
                    0,
                    0,
                    RevisionSide::Baseline,
                    PairOrder::BaselineFirst,
                    0,
                    100,
                ),
                raw(
                    1,
                    1,
                    0,
                    RevisionSide::Candidate,
                    PairOrder::CandidateFirst,
                    0,
                    100,
                ),
            ])
            .unwrap();
        let analysis = analyze_complete_pairs(&mut samples, &ComparisonConfig::default()).unwrap();
        assert!(
            analysis
                .comparisons
                .iter()
                .all(|comparison| comparison.classification == Verdict::InvalidComparison)
        );
    }
}
