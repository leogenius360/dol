//! Canonical single-revision workload runner.

use std::collections::BTreeMap;
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

use crate::artifact::{ArtifactBundle, ArtifactError, Metadata, SummaryArtifact};
use crate::comparison::{DEFAULT_BOOTSTRAP_SEED, summarize_valid_samples};
use crate::sample::{OrderedRawSamples, PairOrder, RawSample, RevisionSide};
use crate::sampling::{SamplingConfiguration, SamplingProfile};
use crate::scenario::ScenarioDefinition;
use crate::statistics::describe;

const CALIBRATION_FLOOR: Duration = Duration::from_millis(10);
const MAX_ITERATIONS: u64 = 50_000_000;

/// Parsed canonical benchmark invocation.
#[derive(Clone, Debug, PartialEq)]
pub struct CanonicalConfig {
    /// Frozen sampling profile.
    pub profile: SamplingProfile,
    /// Optional final/per-side artifact directory.
    pub artifact_dir: Option<PathBuf>,
    /// Optional exact stable scenario ID (never a display-name filter).
    pub scenario_filter: Option<String>,
    /// Revision side. `--side single` aliases candidate for standalone profiling.
    pub side: RevisionSide,
    /// Temporal pair index.
    pub pair: u32,
    /// Complete-pair attempt.
    pub attempt: u32,
    /// Execution order of both sides.
    pub pair_order: PairOrder,
    /// Initial insertion order (normally zero for per-side artifacts).
    pub order_start: u64,
    /// Optional orchestrator-supplied source revision.
    pub source_sha: Option<String>,
    /// Optional orchestrator-supplied fixture manifest digest.
    pub fixture_hash: Option<String>,
    /// Optional orchestrator-supplied original lockfile digest.
    pub lockfile_hash: Option<String>,
    /// Optional validated/local runner identity.
    pub runner: Option<String>,
}

impl Default for CanonicalConfig {
    fn default() -> Self {
        Self {
            profile: SamplingProfile::Adaptive,
            artifact_dir: None,
            scenario_filter: None,
            side: RevisionSide::Candidate,
            pair: 0,
            attempt: 0,
            pair_order: PairOrder::BaselineFirst,
            order_start: 0,
            source_sha: None,
            fixture_hash: None,
            lockfile_hash: None,
            runner: None,
        }
    }
}

impl CanonicalConfig {
    /// Parses canonical options, including Cargo's harmless `--bench` marker.
    pub fn parse(arguments: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut config = Self::default();
        let mut arguments = arguments.into_iter();
        while let Some(argument) = arguments.next() {
            match argument.as_str() {
                "--profile" => {
                    config.profile = next(&mut arguments, "--profile")?.parse()?;
                }
                "--artifact-dir" => {
                    config.artifact_dir =
                        Some(PathBuf::from(next(&mut arguments, "--artifact-dir")?));
                }
                "--scenario" => config.scenario_filter = Some(next(&mut arguments, "--scenario")?),
                "--side" => {
                    config.side = match next(&mut arguments, "--side")?.as_str() {
                        "baseline" => RevisionSide::Baseline,
                        "candidate" | "single" => RevisionSide::Candidate,
                        other => {
                            return Err(format!(
                                "unknown side `{other}`; expected baseline, candidate, or single"
                            ));
                        }
                    };
                }
                "--pair" => config.pair = parse_number(&mut arguments, "--pair")?,
                "--attempt" => config.attempt = parse_number(&mut arguments, "--attempt")?,
                "--pair-order" => {
                    config.pair_order = parse_pair_order(&next(&mut arguments, "--pair-order")?)?;
                }
                "--order" => {
                    let value = next(&mut arguments, "--order")?;
                    if matches!(value.as_str(), "baseline-first" | "candidate-first") {
                        config.pair_order = parse_pair_order(&value)?;
                    } else {
                        config.order_start = value
                            .parse()
                            .map_err(|error| format!("invalid --order value `{value}`: {error}"))?;
                    }
                }
                "--order-start" => {
                    config.order_start = parse_number(&mut arguments, "--order-start")?;
                }
                "--source-sha" => config.source_sha = Some(next(&mut arguments, "--source-sha")?),
                "--fixture-hash" => {
                    config.fixture_hash = Some(next(&mut arguments, "--fixture-hash")?);
                }
                "--lockfile-hash" => {
                    config.lockfile_hash = Some(next(&mut arguments, "--lockfile-hash")?);
                }
                "--runner" => config.runner = Some(next(&mut arguments, "--runner")?),
                // Cargo may append this marker even when harness=false.
                "--bench" => {}
                "--help" | "-h" => return Err(canonical_help().into()),
                other => {
                    return Err(format!(
                        "unknown canonical benchmark option `{other}`\n{}",
                        canonical_help()
                    ));
                }
            }
        }
        if config.attempt > 1 {
            return Err("closure-v1 permits attempt 0 and one retry attempt 1 only".into());
        }
        Ok(config)
    }

    /// Parses process arguments after the binary name.
    pub fn parse_env() -> Result<Self, String> {
        Self::parse(env::args().skip(1))
    }

    fn includes(&self, scenario: ScenarioDefinition) -> bool {
        self.scenario_filter
            .as_ref()
            .is_none_or(|filter| scenario.id == filter)
    }
}

/// Stable canonical CLI help.
#[must_use]
pub const fn canonical_help() -> &'static str {
    "usage: cargo bench -p dol-bench --bench closure_v1 -- \
     [--profile historical-fixed|adaptive|smoke|candidate|release] \
     [--artifact-dir <path>] [--scenario <stable-id>] \
     [--side baseline|candidate|single] [--pair <n>] [--attempt 0|1] \
     [--pair-order baseline-first|candidate-first] [--order-start <n>] \
     [--source-sha <sha>] [--fixture-hash <sha256>] \
     [--lockfile-hash <sha256>] [--runner <id>]"
}

/// Measures canonical operations and retains raw observations in call order.
pub struct CanonicalRunner {
    config: CanonicalConfig,
    sampling: SamplingConfiguration,
    samples: OrderedRawSamples,
    next_order: u64,
}

impl CanonicalRunner {
    /// Creates a runner under one frozen profile.
    #[must_use]
    pub fn new(config: CanonicalConfig) -> Self {
        Self {
            sampling: config.profile.configuration(),
            next_order: config.order_start,
            config,
            samples: OrderedRawSamples::new(),
        }
    }

    /// Whether a stable scenario is selected.
    #[must_use]
    pub fn enabled(&self, scenario: ScenarioDefinition) -> bool {
        self.config.includes(scenario)
    }

    /// Measures one frozen scenario. Setup captured outside `operation` is excluded.
    pub fn measure<T>(
        &mut self,
        scenario: ScenarioDefinition,
        historical_iterations: u64,
        work_units: u64,
        mut operation: impl FnMut() -> T,
    ) -> Result<(), String> {
        if !self.enabled(scenario) {
            return Ok(());
        }
        if work_units == 0 {
            return Err(format!(
                "scenario `{}` declared zero work units",
                scenario.id
            ));
        }

        if !self.sampling.fixed_iterations {
            warm_up(&mut operation, self.sampling.warmup());
        }
        let iterations = if self.sampling.fixed_iterations {
            historical_iterations.max(1)
        } else {
            calibrate(&mut operation, self.sampling.target_sample())
        };

        let start_index = self.samples.len();
        for sample_index in 0..self.sampling.samples {
            let started = Instant::now();
            for _ in 0..iterations {
                black_box(operation());
            }
            let elapsed_ns = u64::try_from(started.elapsed().as_nanos())
                .unwrap_or(u64::MAX)
                .max(1);
            let raw = RawSample::new(
                self.config.pair,
                self.config.attempt,
                self.config.side,
                self.config.pair_order,
                self.next_order,
                scenario.scenario_id(),
                scenario.lifecycle,
                u32::try_from(sample_index)
                    .map_err(|_| "sample index does not fit u32".to_owned())?,
                iterations,
                elapsed_ns,
                work_units,
                scenario.work_unit_kind,
            )
            .map_err(|error| error.to_string())?;
            self.samples.push(raw).map_err(|error| error.to_string())?;
            self.next_order = self
                .next_order
                .checked_add(1)
                .ok_or_else(|| "raw observation order overflow".to_owned())?;
        }

        let end_index = self.samples.len();
        let values = self.samples.as_slice()[start_index..end_index]
            .iter()
            .map(|sample| sample.ns_per_op)
            .collect::<Vec<_>>();
        let (_, classes) = describe(&values).map_err(|error| error.to_string())?;
        for (sample, class) in self.samples.as_mut_slice()[start_index..end_index]
            .iter_mut()
            .zip(classes)
        {
            sample.tukey_class = class;
        }
        Ok(())
    }

    /// Builds an in-memory per-side bundle.
    pub fn finish(self) -> Result<ArtifactBundle, String> {
        if self.samples.is_empty() {
            return Err("no canonical scenarios matched the requested filter".into());
        }
        let metadata = capture_metadata(&self.config).map_err(|error| error.to_string())?;
        let summaries =
            summarize_valid_samples(self.samples.as_slice()).map_err(|error| error.to_string())?;
        Ok(ArtifactBundle::new(
            metadata,
            self.samples,
            SummaryArtifact::new(summaries),
            None,
        ))
    }

    /// Writes artifacts when configured, otherwise returns a human report.
    pub fn finish_and_emit(self) -> Result<(), String> {
        let artifact_dir = self.config.artifact_dir.clone();
        let bundle = self.finish()?;
        if let Some(directory) = artifact_dir {
            let directory = if directory.is_absolute() {
                directory
            } else {
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../..")
                    .join(directory)
            };
            bundle
                .write_to(&directory)
                .map_err(|error| error.to_string())?;
            eprintln!(
                "canonical performance artifacts written to {}",
                directory.display()
            );
        } else {
            print!("{}", render_human(&bundle));
        }
        Ok(())
    }
}

/// Captures required local provenance, accepting orchestrator values where supplied.
pub fn capture_metadata(config: &CanonicalConfig) -> Result<Metadata, ArtifactError> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let rustc_vv = command_output(&root, "rustc", &["-Vv"]);
    let target = rustc_vv
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .unwrap_or("unknown-target")
        .to_owned();
    let mut metadata = Metadata::new(config.profile, DEFAULT_BOOTSTRAP_SEED);
    metadata.runner = config
        .runner
        .clone()
        .or_else(|| env::var("DOL_PERF_RUNNER_ID").ok())
        .unwrap_or_else(|| "local-unqualified".into());
    metadata.source_sha = config
        .source_sha
        .clone()
        .unwrap_or_else(|| command_output(&root, "git", &["rev-parse", "HEAD"]));
    metadata.fixture_hash = config.fixture_hash.clone().unwrap_or_else(|| {
        sha256_normalized_text(&root.join("perf/fixtures/closure-v1/MANIFEST.sha256"))
            .unwrap_or_else(|_| "fixture-manifest-unavailable".into())
    });
    metadata.lockfile_hash = config.lockfile_hash.clone().unwrap_or_else(|| {
        sha256_file(&root.join("Cargo.lock")).unwrap_or_else(|_| "lockfile-unavailable".into())
    });
    metadata.rustc_vv = rustc_vv;
    metadata.cargo_version = command_output(&root, "cargo", &["-V"]);
    metadata.target = target;
    metadata.rustflags = env::var("RUSTFLAGS").unwrap_or_else(|_| "none".into());
    metadata.cargo_profile_overrides = env::vars()
        .filter(|(key, _)| key.starts_with("CARGO_PROFILE_BENCH_"))
        .collect();
    metadata.cpu_topology = env::var("PROCESSOR_IDENTIFIER")
        .or_else(|_| env::var("HOSTTYPE"))
        .unwrap_or_else(|_| "not-captured".into());
    metadata.affinity = env::var("DOL_PERF_AFFINITY").unwrap_or_else(|_| "not-pinned".into());
    metadata.smt = env::var("DOL_PERF_SMT").unwrap_or_else(|_| "not-captured".into());
    metadata.governor = env::var("DOL_PERF_GOVERNOR").unwrap_or_else(|_| "not-captured".into());
    metadata.boost = env::var("DOL_PERF_BOOST").unwrap_or_else(|_| "not-captured".into());
    metadata.numa = env::var("DOL_PERF_NUMA").unwrap_or_else(|_| "not-captured".into());
    metadata.kernel = if cfg!(windows) {
        command_output(&root, "cmd", &["/C", "ver"])
    } else {
        command_output(&root, "uname", &["-srmo"])
    };
    metadata.microcode = env::var("DOL_PERF_MICROCODE").ok();
    metadata.environment_identity = [
        ("os_family".into(), env::consts::FAMILY.into()),
        ("os".into(), env::consts::OS.into()),
        ("arch".into(), env::consts::ARCH.into()),
    ]
    .into_iter()
    .collect::<BTreeMap<_, _>>();
    metadata.timestamp_unix_seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    metadata
        .provenance
        .insert("pair".into(), config.pair.to_string());
    metadata
        .provenance
        .insert("attempt".into(), config.attempt.to_string());
    metadata
        .provenance
        .insert("pair_order".into(), config.pair_order.as_str().to_owned());
    metadata.validate()?;
    Ok(metadata)
}

fn render_human(bundle: &ArtifactBundle) -> String {
    let mut output = format!(
        "DOL closure-v1 canonical benchmark ({})\n",
        bundle.metadata.profile
    );
    for summary in &bundle.summary.scenarios {
        let _ = writeln!(
            output,
            "{} [{}] {:?}: median {:.3} ns/op; p10 {:.3}; p90 {:.3}; NMAD {:.4}; CV {:.4}",
            summary.scenario_id,
            summary.lifecycle,
            summary.side,
            summary.statistics.median,
            summary.statistics.p10,
            summary.statistics.p90,
            summary.statistics.normalized_mad,
            summary.statistics.coefficient_of_variation,
        );
    }
    output
}

fn warm_up<T>(operation: &mut impl FnMut() -> T, duration: Duration) {
    if duration.is_zero() {
        return;
    }
    let started = Instant::now();
    while started.elapsed() < duration {
        black_box(operation());
    }
}

fn calibrate<T>(operation: &mut impl FnMut() -> T, target: Duration) -> u64 {
    let mut iterations = 1_u64;
    loop {
        let started = Instant::now();
        for _ in 0..iterations {
            black_box(operation());
        }
        let elapsed = started.elapsed();
        if elapsed >= CALIBRATION_FLOOR || iterations == MAX_ITERATIONS {
            let scaled = u128::from(iterations)
                .saturating_mul(target.as_nanos())
                .div_ceil(elapsed.as_nanos().max(1));
            return u64::try_from(scaled)
                .unwrap_or(MAX_ITERATIONS)
                .clamp(1, MAX_ITERATIONS);
        }
        iterations = iterations.saturating_mul(10).min(MAX_ITERATIONS);
    }
}

fn command_output(root: &Path, program: &str, arguments: &[&str]) -> String {
    Command::new(program)
        .current_dir(root)
        .args(arguments)
        .output()
        .map_or_else(
            |error| format!("unavailable ({error})"),
            |output| {
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
                if stdout.is_empty() {
                    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
                    if stderr.is_empty() {
                        "unavailable (empty output)".into()
                    } else {
                        stderr
                    }
                } else {
                    stdout
                }
            },
        )
}

fn sha256_file(path: &Path) -> Result<String, std::io::Error> {
    let contents = fs::read(path)?;
    Ok(render_sha256(&contents))
}

fn sha256_normalized_text(path: &Path) -> Result<String, std::io::Error> {
    let contents = fs::read_to_string(path)?.replace("\r\n", "\n");
    Ok(render_sha256(contents.as_bytes()))
}

fn render_sha256(contents: &[u8]) -> String {
    let digest = Sha256::digest(contents);
    let mut rendered = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(rendered, "{byte:02x}");
    }
    rendered
}

fn next(arguments: &mut impl Iterator<Item = String>, option: &str) -> Result<String, String> {
    arguments
        .next()
        .ok_or_else(|| format!("{option} requires a value"))
}

fn parse_number<T>(arguments: &mut impl Iterator<Item = String>, option: &str) -> Result<T, String>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    let value = next(arguments, option)?;
    value
        .parse()
        .map_err(|error| format!("invalid {option} value `{value}`: {error}"))
}

fn parse_pair_order(value: &str) -> Result<PairOrder, String> {
    match value {
        "baseline-first" => Ok(PairOrder::BaselineFirst),
        "candidate-first" => Ok(PairOrder::CandidateFirst),
        other => Err(format!(
            "unknown pair order `{other}`; expected baseline-first or candidate-first"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenario::{Lifecycle, ScenarioTier, WorkUnitKind};

    const TEST_SCENARIO: ScenarioDefinition = ScenarioDefinition {
        id: "test.operation",
        lifecycle: Lifecycle::SteadyState,
        display_name: "Test operation",
        work_unit_kind: WorkUnitKind::Operation,
        tier: ScenarioTier::Canonical,
    };

    #[test]
    fn canonical_options_include_complete_pair_attempt_and_single_alias() {
        let config = CanonicalConfig::parse(
            [
                "--profile",
                "smoke",
                "--side",
                "single",
                "--pair",
                "4",
                "--attempt",
                "1",
                "--order",
                "candidate-first",
                "--scenario",
                "wire.decode",
            ]
            .map(str::to_owned),
        )
        .unwrap();
        assert_eq!(config.profile, SamplingProfile::Smoke);
        assert_eq!(config.side, RevisionSide::Candidate);
        assert_eq!(config.pair, 4);
        assert_eq!(config.attempt, 1);
        assert_eq!(config.pair_order, PairOrder::CandidateFirst);
        assert_eq!(config.scenario_filter.as_deref(), Some("wire.decode"));
    }

    #[test]
    fn historical_fixed_runner_retains_call_order() {
        let config = CanonicalConfig {
            profile: SamplingProfile::HistoricalFixed,
            source_sha: Some("test-source".into()),
            fixture_hash: Some("test-fixture".into()),
            lockfile_hash: Some("test-lock".into()),
            runner: Some("test-runner".into()),
            ..CanonicalConfig::default()
        };
        let mut runner = CanonicalRunner::new(config);
        runner.measure(TEST_SCENARIO, 2, 1, || 42).unwrap();
        assert_eq!(runner.samples.len(), 1);
        assert_eq!(runner.samples.as_slice()[0].order, 0);
        assert_eq!(runner.samples.as_slice()[0].iterations, 2);
    }

    #[test]
    fn scenario_filter_matches_exact_stable_id_across_lifecycles() {
        let config = CanonicalConfig {
            scenario_filter: Some("expr.identity".into()),
            ..CanonicalConfig::default()
        };
        assert!(config.includes(crate::scenario::CLOSURE_V1_SCENARIOS[3]));
        assert!(config.includes(crate::scenario::CLOSURE_V1_SCENARIOS[4]));
        assert!(!config.includes(crate::scenario::CLOSURE_V1_SCENARIOS[19]));
    }
}
