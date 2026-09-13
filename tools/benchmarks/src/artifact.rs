//! Strict `dol-perf/v1` artifact serialization.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::comparison::{
    BootstrapSeed, ComparisonAnalysis, DecisionRules, PairedComparison, RetryStatus,
    ScenarioSummary, Verdict, classify_estimate, summarize_valid_samples,
};
use crate::sample::{OrderedRawSamples, PairOrder, RawSample, RevisionSide};
use crate::sampling::SamplingProfile;
use crate::scenario::{Lifecycle, ScenarioId, WorkUnitKind};
use crate::statistics::TukeyClass;
use crate::{PERFORMANCE_CONTRACT, PERFORMANCE_SCHEMA};

/// Required metadata/provenance artifact.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Metadata {
    /// Artifact schema.
    pub schema: String,
    /// Workload/statistical contract.
    pub contract: String,
    /// Validated runner identifier, or an explicit non-authoritative local ID.
    pub runner: String,
    /// Source revision represented by this single-side run.
    pub source_sha: String,
    /// Revision-neutral closure fixture digest.
    pub fixture_hash: String,
    /// Original revision lockfile digest.
    pub lockfile_hash: String,
    /// Full `rustc -Vv` output.
    pub rustc_vv: String,
    /// `cargo -V` output.
    pub cargo_version: String,
    /// Rust compilation target.
    pub target: String,
    /// Frozen sampling profile.
    pub profile: SamplingProfile,
    /// Effective Rust flags.
    pub rustflags: String,
    /// Effective Cargo profile environment overrides.
    pub cargo_profile_overrides: BTreeMap<String, String>,
    /// CPU/topology identity.
    pub cpu_topology: String,
    /// Applied CPU affinity.
    pub affinity: String,
    /// Simultaneous multithreading state.
    pub smt: String,
    /// CPU frequency governor.
    pub governor: String,
    /// CPU boost state.
    pub boost: String,
    /// NUMA identity/policy.
    pub numa: String,
    /// Kernel/operating-system identity.
    pub kernel: String,
    /// Microcode identity, when available.
    pub microcode: Option<String>,
    /// Stable environment identity fields.
    pub environment_identity: BTreeMap<String, String>,
    /// Unix timestamp in seconds.
    pub timestamp_unix_seconds: u64,
    /// Fixed-width deterministic bootstrap seed.
    pub bootstrap_seed: BootstrapSeed,
    /// Additional provenance (baseline/candidate, runner manifest, overlay, order seed).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub provenance: BTreeMap<String, String>,
}

impl Metadata {
    /// Starts a public, mergeable metadata value with schema/contract/profile fixed.
    #[must_use]
    pub fn new(profile: SamplingProfile, bootstrap_seed: BootstrapSeed) -> Self {
        Self {
            schema: PERFORMANCE_SCHEMA.into(),
            contract: PERFORMANCE_CONTRACT.into(),
            runner: String::new(),
            source_sha: String::new(),
            fixture_hash: String::new(),
            lockfile_hash: String::new(),
            rustc_vv: String::new(),
            cargo_version: String::new(),
            target: String::new(),
            profile,
            rustflags: String::new(),
            cargo_profile_overrides: BTreeMap::new(),
            cpu_topology: String::new(),
            affinity: String::new(),
            smt: String::new(),
            governor: String::new(),
            boost: String::new(),
            numa: String::new(),
            kernel: String::new(),
            microcode: None,
            environment_identity: BTreeMap::new(),
            timestamp_unix_seconds: 0,
            bootstrap_seed,
            provenance: BTreeMap::new(),
        }
    }

    /// Validates required identity fields before evidence is emitted or consumed.
    pub fn validate(&self) -> Result<(), ArtifactError> {
        validate_header(&self.schema, &self.contract)?;
        for (name, value) in [
            ("runner", self.runner.as_str()),
            ("source_sha", self.source_sha.as_str()),
            ("fixture_hash", self.fixture_hash.as_str()),
            ("lockfile_hash", self.lockfile_hash.as_str()),
            ("rustc_vv", self.rustc_vv.as_str()),
            ("cargo_version", self.cargo_version.as_str()),
            ("target", self.target.as_str()),
            ("cpu_topology", self.cpu_topology.as_str()),
            ("affinity", self.affinity.as_str()),
            ("smt", self.smt.as_str()),
            ("governor", self.governor.as_str()),
            ("boost", self.boost.as_str()),
            ("numa", self.numa.as_str()),
            ("kernel", self.kernel.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(ArtifactError::new(format!(
                    "metadata field `{name}` must not be empty"
                )));
            }
        }
        validate_lower_hex(&self.source_sha, 40, "source_sha")?;
        validate_lower_hex(&self.fixture_hash, 64, "fixture_hash")?;
        validate_lower_hex(&self.lockfile_hash, 64, "lockfile_hash")?;
        if self.timestamp_unix_seconds == 0 {
            return Err(ArtifactError::new(
                "metadata timestamp_unix_seconds must be non-zero",
            ));
        }
        Ok(())
    }
}

/// Single-side descriptive-statistics artifact.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SummaryArtifact {
    /// Artifact schema.
    pub schema: String,
    /// Workload/statistical contract.
    pub contract: String,
    /// Ordered scenario summaries.
    pub scenarios: Vec<ScenarioSummary>,
}

impl SummaryArtifact {
    /// Constructs a contract-stamped summary artifact.
    #[must_use]
    pub fn new(scenarios: Vec<ScenarioSummary>) -> Self {
        Self {
            schema: PERFORMANCE_SCHEMA.into(),
            contract: PERFORMANCE_CONTRACT.into(),
            scenarios,
        }
    }

    /// Checks headers and all floating-point fields.
    pub fn validate(&self) -> Result<(), ArtifactError> {
        validate_header(&self.schema, &self.contract)?;
        for scenario in &self.scenarios {
            validate_statistics(&scenario.statistics)?;
        }
        Ok(())
    }
}

/// Paired decision artifact.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PairedComparisonArtifact {
    /// Artifact schema.
    pub schema: String,
    /// Workload/statistical contract.
    pub contract: String,
    /// Immutable reference revision.
    pub baseline_sha: String,
    /// Immutable candidate revision.
    pub candidate_sha: String,
    /// Scenario decisions.
    pub comparisons: Vec<PairedComparison>,
}

impl PairedComparisonArtifact {
    /// Constructs an artifact from a completed analysis.
    #[must_use]
    pub fn new(
        baseline_sha: impl Into<String>,
        candidate_sha: impl Into<String>,
        comparisons: Vec<PairedComparison>,
    ) -> Self {
        Self {
            schema: PERFORMANCE_SCHEMA.into(),
            contract: PERFORMANCE_CONTRACT.into(),
            baseline_sha: baseline_sha.into(),
            candidate_sha: candidate_sha.into(),
            comparisons,
        }
    }

    /// Checks identity, headers, and all estimates.
    pub fn validate(&self) -> Result<(), ArtifactError> {
        validate_header(&self.schema, &self.contract)?;
        if self.baseline_sha.trim().is_empty() || self.candidate_sha.trim().is_empty() {
            return Err(ArtifactError::new(
                "paired comparison requires baseline and candidate SHAs",
            ));
        }
        if self.comparisons.is_empty() {
            return Err(ArtifactError::new(
                "paired comparison must contain at least one scenario",
            ));
        }
        let mut keys = BTreeSet::new();
        for comparison in &self.comparisons {
            if !keys.insert((comparison.scenario_id.clone(), comparison.lifecycle)) {
                return Err(ArtifactError::new(format!(
                    "duplicate paired comparison for `{}` / `{}`",
                    comparison.scenario_id, comparison.lifecycle
                )));
            }
            if comparison.reason.trim().is_empty() {
                return Err(ArtifactError::new(format!(
                    "scenario `{}` has an empty comparison reason",
                    comparison.scenario_id
                )));
            }
            if let Some(estimate) = &comparison.estimate {
                let mut values = estimate.paired_ratios.clone();
                values.extend([
                    estimate.median_ratio,
                    estimate.confidence_interval_95.lower,
                    estimate.confidence_interval_95.upper,
                ]);
                if estimate.resamples != crate::comparison::BOOTSTRAP_RESAMPLES {
                    return Err(ArtifactError::new(format!(
                        "scenario `{}` has a non-contract bootstrap count",
                        comparison.scenario_id
                    )));
                }
                if !values.into_iter().all(f64::is_finite) {
                    return Err(ArtifactError::new(format!(
                        "scenario `{}` contains a non-finite estimate",
                        comparison.scenario_id
                    )));
                }
                if estimate.paired_ratios.len() != comparison.pair_count {
                    return Err(ArtifactError::new(format!(
                        "scenario `{}` pair count does not match its paired ratios",
                        comparison.scenario_id
                    )));
                }
                if comparison.pair_count == 0
                    || estimate.paired_ratios.iter().any(|ratio| *ratio <= -1.0)
                {
                    return Err(ArtifactError::new(format!(
                        "scenario `{}` has an impossible paired cost ratio",
                        comparison.scenario_id
                    )));
                }
                if matches!(
                    comparison.retry_status,
                    RetryStatus::Required | RetryStatus::Exhausted
                ) {
                    return Err(ArtifactError::new(format!(
                        "scenario `{}` has an estimate while retry work is unresolved",
                        comparison.scenario_id
                    )));
                }
                if estimate.confidence_interval_95.lower > estimate.confidence_interval_95.upper {
                    return Err(ArtifactError::new(format!(
                        "scenario `{}` has a reversed confidence interval",
                        comparison.scenario_id
                    )));
                }
                let (expected, _) = classify_estimate(estimate, DecisionRules::default());
                if comparison.classification != expected {
                    return Err(ArtifactError::new(format!(
                        "scenario `{}` classification does not match closure-v1 decision rules",
                        comparison.scenario_id
                    )));
                }
            } else if matches!(
                comparison.classification,
                Verdict::Improvement
                    | Verdict::Neutral
                    | Verdict::Warning
                    | Verdict::HardRegression
            ) {
                return Err(ArtifactError::new(format!(
                    "scenario `{}` has a threshold verdict without an estimate",
                    comparison.scenario_id
                )));
            }
        }
        Ok(())
    }
}

/// Complete in-memory artifact bundle. Per-side runs omit `paired_comparison`;
/// final A/B and A/A evidence includes it.
#[derive(Clone, Debug, PartialEq)]
pub struct ArtifactBundle {
    /// Identity and provenance.
    pub metadata: Metadata,
    /// Insertion-ordered observations, including rejected attempts.
    pub raw_samples: OrderedRawSamples,
    /// Single-side descriptions.
    pub summary: SummaryArtifact,
    /// Final decision layer, if this is a composed comparison bundle.
    pub paired_comparison: Option<PairedComparisonArtifact>,
}

impl ArtifactBundle {
    /// Public composition constructor for xtask orchestration.
    #[must_use]
    pub const fn new(
        metadata: Metadata,
        raw_samples: OrderedRawSamples,
        summary: SummaryArtifact,
        paired_comparison: Option<PairedComparisonArtifact>,
    ) -> Self {
        Self {
            metadata,
            raw_samples,
            summary,
            paired_comparison,
        }
    }

    /// Creates a final bundle from the mutable raw evidence and final analysis.
    #[must_use]
    pub fn from_analysis(
        metadata: Metadata,
        raw_samples: OrderedRawSamples,
        analysis: ComparisonAnalysis,
        baseline_sha: impl Into<String>,
        candidate_sha: impl Into<String>,
    ) -> Self {
        Self {
            metadata,
            raw_samples,
            summary: SummaryArtifact::new(analysis.summaries),
            paired_comparison: Some(PairedComparisonArtifact::new(
                baseline_sha,
                candidate_sha,
                analysis.comparisons,
            )),
        }
    }

    /// Validates cross-file and per-file invariants.
    pub fn validate(&self) -> Result<(), ArtifactError> {
        self.metadata.validate()?;
        self.raw_samples
            .validate()
            .map_err(|error| ArtifactError::new(error.to_string()))?;
        self.summary.validate()?;
        let expected_summary = summarize_valid_samples(self.raw_samples.as_slice())
            .map_err(|error| ArtifactError::new(error.to_string()))?;
        if self.summary.scenarios != expected_summary {
            return Err(ArtifactError::new(
                "summary.json does not exactly describe the comparison-valid raw samples",
            ));
        }
        if let Some(comparison) = &self.paired_comparison {
            comparison.validate()?;
            if self.metadata.source_sha != comparison.candidate_sha {
                return Err(ArtifactError::new(
                    "metadata source_sha does not match paired candidate_sha",
                ));
            }
            for (key, expected) in [
                ("baseline_sha", comparison.baseline_sha.as_str()),
                ("candidate_sha", comparison.candidate_sha.as_str()),
            ] {
                if self.metadata.provenance.get(key).map(String::as_str) != Some(expected) {
                    return Err(ArtifactError::new(format!(
                        "metadata provenance `{key}` is missing or disagrees with paired comparison"
                    )));
                }
            }
            let raw_keys = self
                .raw_samples
                .as_slice()
                .iter()
                .map(|sample| (sample.scenario_id.clone(), sample.lifecycle))
                .collect::<BTreeSet<_>>();
            let comparison_keys = comparison
                .comparisons
                .iter()
                .map(|entry| (entry.scenario_id.clone(), entry.lifecycle))
                .collect::<BTreeSet<_>>();
            if raw_keys != comparison_keys {
                return Err(ArtifactError::new(
                    "paired-comparison.json scenario keys do not match raw-samples.csv",
                ));
            }
            for entry in &comparison.comparisons {
                if entry
                    .estimate
                    .as_ref()
                    .is_some_and(|estimate| estimate.seed != self.metadata.bootstrap_seed)
                {
                    return Err(ArtifactError::new(format!(
                        "scenario `{}` bootstrap seed disagrees with metadata",
                        entry.scenario_id
                    )));
                }
            }
        }
        Ok(())
    }

    /// Writes canonical filenames beneath `directory`.
    pub fn write_to(&self, directory: impl AsRef<Path>) -> Result<(), ArtifactError> {
        self.validate()?;
        let directory = directory.as_ref();
        fs::create_dir_all(directory).map_err(|error| io_error(directory, error))?;
        write_json(directory.join("metadata.json"), &self.metadata)?;
        write_raw_samples(directory.join("raw-samples.csv"), &self.raw_samples)?;
        write_json(directory.join("summary.json"), &self.summary)?;
        let comparison_path = directory.join("paired-comparison.json");
        if let Some(comparison) = &self.paired_comparison {
            write_json(comparison_path, comparison)?;
        } else if comparison_path.exists() {
            fs::remove_file(&comparison_path).map_err(|error| io_error(&comparison_path, error))?;
        }
        Ok(())
    }

    /// Reads canonical filenames and strictly validates the result.
    pub fn read_from(directory: impl AsRef<Path>) -> Result<Self, ArtifactError> {
        let directory = directory.as_ref();
        let metadata = read_json(directory.join("metadata.json"))?;
        let raw_samples = read_raw_samples(directory.join("raw-samples.csv"))?;
        let summary = read_json(directory.join("summary.json"))?;
        let comparison_path = directory.join("paired-comparison.json");
        let paired_comparison = comparison_path
            .is_file()
            .then(|| read_json(comparison_path))
            .transpose()?;
        let bundle = Self {
            metadata,
            raw_samples,
            summary,
            paired_comparison,
        };
        bundle.validate()?;
        Ok(bundle)
    }
}

const CSV_HEADER: &[&str] = &[
    "pair",
    "attempt",
    "side",
    "pair_order",
    "order",
    "scenario_id",
    "lifecycle",
    "sample",
    "iterations",
    "elapsed_ns",
    "ns_per_op",
    "throughput",
    "work_units",
    "work_unit_kind",
    "tukey_class",
    "valid_for_comparison",
    "rejection_reason",
];

/// Writes observations in insertion order; no sort or outlier removal occurs.
pub fn write_raw_samples(
    path: impl AsRef<Path>,
    samples: &OrderedRawSamples,
) -> Result<(), ArtifactError> {
    samples
        .validate()
        .map_err(|error| ArtifactError::new(error.to_string()))?;
    let mut output = String::new();
    csv_record(
        &mut output,
        &CSV_HEADER
            .iter()
            .map(|value| (*value).to_owned())
            .collect::<Vec<_>>(),
    );
    for sample in samples.as_slice() {
        csv_record(
            &mut output,
            &[
                sample.pair.to_string(),
                sample.attempt.to_string(),
                sample.side.as_str().into(),
                sample.pair_order.as_str().into(),
                sample.order.to_string(),
                sample.scenario_id.to_string(),
                sample.lifecycle.as_str().into(),
                sample.sample.to_string(),
                sample.iterations.to_string(),
                sample.elapsed_ns.to_string(),
                sample.ns_per_op.to_string(),
                sample.throughput.to_string(),
                sample.work_units.to_string(),
                sample.work_unit_kind.as_str().into(),
                sample.tukey_class.as_str().into(),
                sample.valid_for_comparison.to_string(),
                sample.rejection_reason.clone().unwrap_or_default(),
            ],
        );
    }
    let path = path.as_ref();
    fs::write(path, output).map_err(|error| io_error(path, error))
}

/// Reads and validates observations without changing row order.
pub fn read_raw_samples(path: impl AsRef<Path>) -> Result<OrderedRawSamples, ArtifactError> {
    let path = path.as_ref();
    let input = fs::read_to_string(path).map_err(|error| io_error(path, error))?;
    let records = parse_csv(&input)?;
    let (header, rows) = records
        .split_first()
        .ok_or_else(|| ArtifactError::new("raw-samples.csv is empty"))?;
    if header
        .iter()
        .map(String::as_str)
        .ne(CSV_HEADER.iter().copied())
    {
        return Err(ArtifactError::new(format!(
            "raw-samples.csv header does not match {}",
            CSV_HEADER.join(",")
        )));
    }
    let mut samples = Vec::with_capacity(rows.len());
    for (row_index, fields) in rows.iter().enumerate() {
        if fields.len() != CSV_HEADER.len() {
            return Err(ArtifactError::new(format!(
                "raw-samples.csv row {} has {} fields; expected {}",
                row_index + 2,
                fields.len(),
                CSV_HEADER.len()
            )));
        }
        let field = |index: usize| fields[index].as_str();
        let sample = RawSample {
            pair: parse(field(0), "pair", row_index)?,
            attempt: parse(field(1), "attempt", row_index)?,
            side: parse_side(field(2))?,
            pair_order: parse_pair_order(field(3))?,
            order: parse(field(4), "order", row_index)?,
            scenario_id: ScenarioId::new(field(5)).map_err(ArtifactError::new)?,
            lifecycle: parse_lifecycle(field(6))?,
            sample: parse(field(7), "sample", row_index)?,
            iterations: parse(field(8), "iterations", row_index)?,
            elapsed_ns: parse(field(9), "elapsed_ns", row_index)?,
            ns_per_op: parse(field(10), "ns_per_op", row_index)?,
            throughput: parse(field(11), "throughput", row_index)?,
            work_units: parse(field(12), "work_units", row_index)?,
            work_unit_kind: parse_work_unit(field(13))?,
            tukey_class: parse_tukey(field(14))?,
            valid_for_comparison: parse(field(15), "valid_for_comparison", row_index)?,
            rejection_reason: (!field(16).is_empty()).then(|| field(16).to_owned()),
        };
        sample
            .validate()
            .map_err(|error| ArtifactError::new(error.to_string()))?;
        samples.push(sample);
    }
    OrderedRawSamples::from_vec(samples).map_err(|error| ArtifactError::new(error.to_string()))
}

fn write_json(path: PathBuf, value: &impl Serialize) -> Result<(), ArtifactError> {
    let mut bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| ArtifactError::new(format!("serialize {}: {error}", path.display())))?;
    bytes.push(b'\n');
    fs::write(&path, bytes).map_err(|error| io_error(&path, error))
}

fn read_json<T: DeserializeOwned>(path: PathBuf) -> Result<T, ArtifactError> {
    let bytes = fs::read(&path).map_err(|error| io_error(&path, error))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| ArtifactError::new(format!("parse {}: {error}", path.display())))
}

fn validate_header(schema: &str, contract: &str) -> Result<(), ArtifactError> {
    if schema != PERFORMANCE_SCHEMA {
        return Err(ArtifactError::new(format!(
            "unsupported artifact schema `{schema}`; expected `{PERFORMANCE_SCHEMA}`"
        )));
    }
    if contract != PERFORMANCE_CONTRACT {
        return Err(ArtifactError::new(format!(
            "unsupported performance contract `{contract}`; expected `{PERFORMANCE_CONTRACT}`"
        )));
    }
    Ok(())
}

fn validate_lower_hex(value: &str, length: usize, field: &str) -> Result<(), ArtifactError> {
    if value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(ArtifactError::new(format!(
            "metadata field `{field}` must be exactly {length} lowercase hexadecimal characters"
        )))
    }
}

fn validate_statistics(
    statistics: &crate::statistics::DescriptiveStatistics,
) -> Result<(), ArtifactError> {
    let finite = [
        statistics.median,
        statistics.p10,
        statistics.p90,
        statistics.mad,
        statistics.normalized_mad,
        statistics.coefficient_of_variation,
        statistics.min,
        statistics.max,
    ]
    .into_iter()
    .all(f64::is_finite);
    if !finite || statistics.sample_count == 0 {
        return Err(ArtifactError::new(
            "summary contains non-finite values or no observations",
        ));
    }
    Ok(())
}

fn csv_record(output: &mut String, fields: &[String]) {
    for (index, field) in fields.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        output.push('"');
        output.push_str(&field.replace('"', "\"\""));
        output.push('"');
    }
    output.push('\n');
}

fn parse_csv(input: &str) -> Result<Vec<Vec<String>>, ArtifactError> {
    let mut records = Vec::new();
    let mut record = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let bytes = input.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if quoted {
            if byte == b'"' {
                if bytes.get(index + 1) == Some(&b'"') {
                    field.push('"');
                    index += 1;
                } else {
                    quoted = false;
                }
            } else {
                field.push(char::from(byte));
            }
        } else {
            match byte {
                b'"' if field.is_empty() => quoted = true,
                b',' => record.push(std::mem::take(&mut field)),
                b'\n' => {
                    record.push(std::mem::take(&mut field));
                    records.push(std::mem::take(&mut record));
                }
                b'\r' if bytes.get(index + 1) == Some(&b'\n') => {}
                b'\r' => {
                    record.push(std::mem::take(&mut field));
                    records.push(std::mem::take(&mut record));
                }
                _ => field.push(char::from(byte)),
            }
        }
        index += 1;
    }
    if quoted {
        return Err(ArtifactError::new("unterminated quoted CSV field"));
    }
    if !field.is_empty() || !record.is_empty() {
        record.push(field);
        records.push(record);
    }
    Ok(records)
}

fn parse<T>(value: &str, field: &str, row_index: usize) -> Result<T, ArtifactError>
where
    T: std::str::FromStr,
    T::Err: fmt::Display,
{
    value.parse().map_err(|error| {
        ArtifactError::new(format!(
            "invalid `{field}` at raw-samples.csv row {}: {error}",
            row_index + 2
        ))
    })
}

fn parse_side(value: &str) -> Result<RevisionSide, ArtifactError> {
    match value {
        "baseline" => Ok(RevisionSide::Baseline),
        "candidate" => Ok(RevisionSide::Candidate),
        _ => Err(ArtifactError::new(format!(
            "invalid revision side `{value}`"
        ))),
    }
}

fn parse_pair_order(value: &str) -> Result<PairOrder, ArtifactError> {
    match value {
        "baseline-first" => Ok(PairOrder::BaselineFirst),
        "candidate-first" => Ok(PairOrder::CandidateFirst),
        _ => Err(ArtifactError::new(format!("invalid pair order `{value}`"))),
    }
}

fn parse_lifecycle(value: &str) -> Result<Lifecycle, ArtifactError> {
    match value {
        "cold" => Ok(Lifecycle::Cold),
        "warm" => Ok(Lifecycle::Warm),
        "prepared" => Ok(Lifecycle::Prepared),
        "fresh" => Ok(Lifecycle::Fresh),
        "shared-plan" => Ok(Lifecycle::SharedPlan),
        "steady-state" => Ok(Lifecycle::SteadyState),
        _ => Err(ArtifactError::new(format!("invalid lifecycle `{value}`"))),
    }
}

fn parse_work_unit(value: &str) -> Result<WorkUnitKind, ArtifactError> {
    match value {
        "operation" => Ok(WorkUnitKind::Operation),
        "evaluation" => Ok(WorkUnitKind::Evaluation),
        "node" => Ok(WorkUnitKind::Node),
        "stage" => Ok(WorkUnitKind::Stage),
        "field" => Ok(WorkUnitKind::Field),
        "byte" => Ok(WorkUnitKind::Byte),
        "row" => Ok(WorkUnitKind::Row),
        "candidate" => Ok(WorkUnitKind::Candidate),
        _ => Err(ArtifactError::new(format!("invalid work unit `{value}`"))),
    }
}

fn parse_tukey(value: &str) -> Result<TukeyClass, ArtifactError> {
    match value {
        "lower-extreme" => Ok(TukeyClass::LowerExtreme),
        "lower-mild" => Ok(TukeyClass::LowerMild),
        "inlier" => Ok(TukeyClass::Inlier),
        "upper-mild" => Ok(TukeyClass::UpperMild),
        "upper-extreme" => Ok(TukeyClass::UpperExtreme),
        _ => Err(ArtifactError::new(format!("invalid Tukey class `{value}`"))),
    }
}

fn io_error(path: &Path, error: std::io::Error) -> ArtifactError {
    ArtifactError::new(format!("{}: {error}", path.display()))
}

/// Artifact read/write/validation failure.
#[derive(Debug)]
pub struct ArtifactError(String);

impl ArtifactError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for ArtifactError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for ArtifactError {}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;
    use crate::comparison::DEFAULT_BOOTSTRAP_SEED;
    use crate::scenario::WorkUnitKind;

    fn metadata() -> Metadata {
        let mut value = Metadata::new(SamplingProfile::Smoke, DEFAULT_BOOTSTRAP_SEED);
        value.runner = "test-runner".into();
        value.source_sha = "0123456789abcdef0123456789abcdef01234567".into();
        value.fixture_hash =
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".into();
        value.lockfile_hash =
            "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789".into();
        value.rustc_vv = "rustc 1.98.0\nhost: x86_64-test".into();
        value.cargo_version = "cargo 1.98.0".into();
        value.target = "x86_64-test".into();
        value.cpu_topology = "one test CPU".into();
        value.affinity = "0".into();
        value.smt = "false".into();
        value.governor = "test".into();
        value.boost = "false".into();
        value.numa = "0".into();
        value.kernel = "test".into();
        value.timestamp_unix_seconds = 1;
        value
    }

    fn samples() -> OrderedRawSamples {
        let mut values = OrderedRawSamples::new();
        for index in 0..3 {
            values
                .push(
                    RawSample::new(
                        0,
                        0,
                        RevisionSide::Candidate,
                        PairOrder::BaselineFirst,
                        index,
                        ScenarioId::new("wire.decode.v1.84b").unwrap(),
                        Lifecycle::Prepared,
                        u32::try_from(index).unwrap(),
                        10,
                        840 + index,
                        84,
                        WorkUnitKind::Byte,
                    )
                    .unwrap(),
                )
                .unwrap();
        }
        values
    }

    fn temporary_directory() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "dol-bench-artifact-{}-{unique}",
            std::process::id()
        ))
    }

    #[test]
    fn raw_csv_round_trip_preserves_exact_insertion_order_and_quotes() {
        let mut values = samples();
        values.as_mut_slice()[1]
            .reject("noise, \"quoted\"")
            .unwrap();
        let directory = temporary_directory();
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("raw-samples.csv");
        write_raw_samples(&path, &values).unwrap();
        let restored = read_raw_samples(&path).unwrap();
        assert_eq!(restored, values);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn raw_csv_rejects_a_reason_on_a_valid_observation() {
        let directory = temporary_directory();
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("raw-samples.csv");
        write_raw_samples(&path, &samples()).unwrap();
        let source = fs::read_to_string(&path).unwrap();
        let corrupted = source.replacen("\"true\",\"\"", "\"true\",\"unexpected\"", 1);
        fs::write(&path, corrupted).unwrap();
        assert!(read_raw_samples(&path).is_err());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn per_side_bundle_round_trips_with_strict_headers() {
        let values = samples();
        let scenarios = crate::comparison::summarize_valid_samples(values.as_slice()).unwrap();
        let bundle = ArtifactBundle::new(metadata(), values, SummaryArtifact::new(scenarios), None);
        let directory = temporary_directory();
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("paired-comparison.json"), b"stale").unwrap();
        bundle.write_to(&directory).unwrap();
        assert_eq!(ArtifactBundle::read_from(&directory).unwrap(), bundle);
        assert!(!directory.join("paired-comparison.json").exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn schema_and_non_finite_values_are_rejected() {
        let mut invalid_metadata = metadata();
        invalid_metadata.schema = "dol-perf/v2".into();
        assert!(invalid_metadata.validate().is_err());

        let mut stats = crate::comparison::summarize_valid_samples(samples().as_slice())
            .unwrap()
            .remove(0);
        stats.statistics.median = f64::NAN;
        assert!(SummaryArtifact::new(vec![stats]).validate().is_err());

        let values = samples();
        let bundle =
            ArtifactBundle::new(metadata(), values, SummaryArtifact::new(Vec::new()), None);
        assert!(bundle.validate().is_err());
    }
}
