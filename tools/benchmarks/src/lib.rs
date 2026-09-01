#![forbid(unsafe_code)]

//! Small benchmark harness shared by DOL's framework-free benchmark binaries.

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::hint::black_box;
use std::io;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// SHA-256 of the user-attested archive carrying the historical fixture.
pub const HISTORICAL_ARCHIVE_SHA256: &str =
    "66F1631213557B6EB011D1D4CF896DFEC6C392F5B1D286175D6834BAB65FE71C";
/// SHA-256 of `dol/benches/language.rs` inside that archive.
pub const HISTORICAL_HARNESS_SHA256: &str =
    "03B2638B1F469A8F00FC0EF006E69515728290DDFF684F9684C934F2C4CA3306";

const SAMPLE_COUNT: usize = 9;
const CALIBRATION_FLOOR: Duration = Duration::from_millis(20);
const TARGET_SAMPLE: Duration = Duration::from_millis(150);
const MAX_ITERATIONS: u64 = 50_000_000;

/// Benchmark workload selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Suite {
    /// User-attested August 2026 fixture equivalents.
    Historical,
    /// Current roadmap and decomposed workloads.
    Modern,
    /// Both suites.
    All,
}

/// Sampling policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// One pass using the fixture's historical iteration count.
    HistoricalFixed,
    /// Nine calibrated samples targeting approximately 150 ms each.
    Adaptive,
}

/// Output encoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// Human-readable report.
    Text,
    /// Stable comma-separated records.
    Csv,
}

/// Parsed benchmark command-line configuration.
#[derive(Clone, Debug)]
pub struct Config {
    /// Selected workload family.
    pub suite: Suite,
    /// Sampling policy.
    pub mode: Mode,
    /// Case-insensitive workload-name substring.
    pub filter: Option<String>,
    /// Report encoding.
    pub format: Format,
    /// Optional report destination. Standard output is used otherwise.
    pub output: Option<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            suite: Suite::All,
            mode: Mode::Adaptive,
            filter: None,
            format: Format::Text,
            output: None,
        }
    }
}

impl Config {
    /// Parses `--suite`, `--filter`, `--mode`, `--format`, and `--output`.
    pub fn parse_env() -> Result<Self, String> {
        Self::parse_arguments(env::args().skip(1))
    }

    fn parse_arguments(arguments: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut config = Self::default();
        let mut arguments = arguments.into_iter();
        while let Some(argument) = arguments.next() {
            match argument.as_str() {
                "--suite" => {
                    config.suite = match next_value(&mut arguments, "--suite")?.as_str() {
                        "historical" => Suite::Historical,
                        "modern" => Suite::Modern,
                        "all" => Suite::All,
                        other => {
                            return Err(format!(
                                "unknown suite `{other}`; expected historical, modern, or all"
                            ));
                        }
                    };
                }
                "--filter" => config.filter = Some(next_value(&mut arguments, "--filter")?),
                "--mode" => {
                    config.mode = match next_value(&mut arguments, "--mode")?.as_str() {
                        "historical-fixed" => Mode::HistoricalFixed,
                        "adaptive" => Mode::Adaptive,
                        other => {
                            return Err(format!(
                                "unknown mode `{other}`; expected historical-fixed or adaptive"
                            ));
                        }
                    };
                }
                "--format" => {
                    config.format = match next_value(&mut arguments, "--format")?.as_str() {
                        "text" => Format::Text,
                        "csv" => Format::Csv,
                        other => {
                            return Err(format!("unknown format `{other}`; expected text or csv"));
                        }
                    };
                }
                "--output" => {
                    config.output = Some(PathBuf::from(next_value(&mut arguments, "--output")?));
                }
                // Cargo appends this libtest-compatible marker to bench targets
                // even when `harness = false`.
                "--bench" => {}
                "--help" | "-h" => return Err(help().into()),
                other => return Err(format!("unknown benchmark option `{other}`\n{}", help())),
            }
        }
        Ok(config)
    }

    fn suite_enabled(&self, suite: Suite) -> bool {
        self.suite == Suite::All || self.suite == suite
    }

    fn includes(&self, suite: Suite, name: &str) -> bool {
        let selected = self.suite_enabled(suite);
        let filtered = self
            .filter
            .as_ref()
            .is_none_or(|filter| name.to_lowercase().contains(&filter.to_lowercase()));
        selected && filtered
    }
}

fn next_value(
    arguments: &mut impl Iterator<Item = String>,
    option: &str,
) -> Result<String, String> {
    arguments
        .next()
        .ok_or_else(|| format!("{option} requires a value"))
}

/// Command-line help for the roadmap benchmark binary.
#[must_use]
pub const fn help() -> &'static str {
    "usage: cargo xtask bench -- [--suite historical|modern|all] \
     [--filter <substring>] [--mode historical-fixed|adaptive] \
     [--format text|csv] [--output <path>]"
}

#[derive(Clone, Debug)]
struct Measurement {
    order: usize,
    suite: Suite,
    name: String,
    operations_per_second: f64,
    nanos_per_operation: f64,
    minimum: f64,
    maximum: f64,
    iterations: u64,
    samples: usize,
}

#[derive(Clone, Debug)]
struct Note {
    order: usize,
    suite: Suite,
    name: String,
    detail: String,
}

/// Collects benchmark results and renders one self-describing report.
pub struct Runner {
    config: Config,
    metadata: Vec<(String, String)>,
    measurements: Vec<Measurement>,
    notes: Vec<Note>,
    next_order: usize,
}

impl Runner {
    /// Creates a report and captures host/toolchain/provenance metadata.
    #[must_use]
    pub fn new(config: Config) -> Self {
        Self {
            config,
            metadata: capture_metadata(),
            measurements: Vec::new(),
            notes: Vec::new(),
            next_order: 0,
        }
    }

    /// Whether a workload would be selected by the current suite and filter.
    #[must_use]
    pub fn enabled(&self, suite: Suite, name: &str) -> bool {
        self.config.includes(suite, name)
    }

    /// Whether a workload family is selected, independently of the name filter.
    #[must_use]
    pub fn suite_enabled(&self, suite: Suite) -> bool {
        self.config.suite_enabled(suite)
    }

    /// Records an informational capability or comparability result.
    pub fn note(&mut self, suite: Suite, name: impl Into<String>, detail: impl Into<String>) {
        let name = name.into();
        if self.enabled(suite, &name) {
            let order = self.next_order;
            self.next_order += 1;
            self.notes.push(Note {
                order,
                suite,
                name,
                detail: detail.into(),
            });
        }
    }

    /// Measures an operation under the selected sampling policy.
    ///
    /// `historical_iterations` is used only in `historical-fixed` mode.
    pub fn measure<T>(
        &mut self,
        suite: Suite,
        name: impl Into<String>,
        historical_iterations: u64,
        mut operation: impl FnMut() -> T,
    ) {
        let name = name.into();
        if !self.enabled(suite, &name) {
            return;
        }

        let (iterations, sample_count) = match self.config.mode {
            Mode::HistoricalFixed => (historical_iterations.max(1), 1),
            Mode::Adaptive => (calibrate(&mut operation), SAMPLE_COUNT),
        };
        let mut samples = Vec::with_capacity(sample_count);
        for _ in 0..sample_count {
            let started = Instant::now();
            for _ in 0..iterations {
                black_box(operation());
            }
            samples.push(started.elapsed().as_secs_f64());
        }
        samples.sort_by(f64::total_cmp);
        let median_seconds = samples[sample_count / 2].max(f64::MIN_POSITIVE);
        let operations_per_second = iterations as f64 / median_seconds;
        let order = self.next_order;
        self.next_order += 1;
        self.measurements.push(Measurement {
            order,
            suite,
            name,
            operations_per_second,
            nanos_per_operation: median_seconds * 1_000_000_000.0 / iterations as f64,
            minimum: iterations as f64 / samples[sample_count - 1].max(f64::MIN_POSITIVE),
            maximum: iterations as f64 / samples[0].max(f64::MIN_POSITIVE),
            iterations,
            samples: sample_count,
        });
    }

    /// Writes the selected report to stdout or `--output`.
    pub fn finish(self) -> io::Result<()> {
        let rendered = match self.config.format {
            Format::Text => self.render_text(),
            Format::Csv => self.render_csv(),
        };
        if let Some(configured_path) = &self.config.output {
            let path = if configured_path.is_absolute() {
                configured_path.clone()
            } else {
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../..")
                    .join(configured_path)
            };
            if let Some(parent) = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            fs::write(&path, rendered)?;
            println!("benchmark report written to {}", path.display());
        } else {
            print!("{rendered}");
        }
        Ok(())
    }

    fn render_text(&self) -> String {
        let mut output = String::from("DOL archive-identified benchmark suite\n");
        let _ = writeln!(
            output,
            "mode: {}; adaptive policy: {SAMPLE_COUNT} samples at approximately {} ms",
            mode_name(self.config.mode),
            TARGET_SAMPLE.as_millis()
        );
        output.push_str("\nMetadata\n");
        for (key, value) in &self.metadata {
            let _ = writeln!(output, "{key}: {value}");
        }
        for suite in [Suite::Historical, Suite::Modern] {
            if !self.measurements.iter().any(|row| row.suite == suite)
                && !self.notes.iter().any(|note| note.suite == suite)
            {
                continue;
            }
            let _ = write!(output, "\n{} suite\n", suite_name(suite));
            for order in 0..self.next_order {
                if let Some(row) = self
                    .measurements
                    .iter()
                    .find(|row| row.suite == suite && row.order == order)
                {
                    let _ = writeln!(
                        output,
                        "{}: {:.0} ops/s ({:.1} ns/op; range {:.0}..{:.0}; iterations={}; samples={})",
                        row.name,
                        row.operations_per_second,
                        row.nanos_per_operation,
                        row.minimum,
                        row.maximum,
                        row.iterations,
                        row.samples
                    );
                } else if let Some(note) = self
                    .notes
                    .iter()
                    .find(|note| note.suite == suite && note.order == order)
                {
                    let _ = writeln!(output, "{}: {}", note.name, note.detail);
                }
            }
        }
        output
    }

    fn render_csv(&self) -> String {
        let mut output = String::from(
            "record,suite,name,ops_per_second,ns_per_operation,min_ops_per_second,max_ops_per_second,iterations,samples,value\n",
        );
        for (key, value) in &self.metadata {
            csv_row(
                &mut output,
                &["metadata", "", key, "", "", "", "", "", "", value],
            );
        }
        for order in 0..self.next_order {
            if let Some(row) = self.measurements.iter().find(|row| row.order == order) {
                csv_row(
                    &mut output,
                    &[
                        "measurement",
                        suite_name(row.suite),
                        &row.name,
                        &format!("{:.6}", row.operations_per_second),
                        &format!("{:.6}", row.nanos_per_operation),
                        &format!("{:.6}", row.minimum),
                        &format!("{:.6}", row.maximum),
                        &row.iterations.to_string(),
                        &row.samples.to_string(),
                        "",
                    ],
                );
            } else if let Some(note) = self.notes.iter().find(|note| note.order == order) {
                csv_row(
                    &mut output,
                    &[
                        "note",
                        suite_name(note.suite),
                        &note.name,
                        "",
                        "",
                        "",
                        "",
                        "",
                        "",
                        &note.detail,
                    ],
                );
            }
        }
        output
    }
}

fn calibrate<T>(operation: &mut impl FnMut() -> T) -> u64 {
    let mut iterations = 1_u64;
    loop {
        let started = Instant::now();
        for _ in 0..iterations {
            black_box(operation());
        }
        let elapsed = started.elapsed();
        if elapsed >= CALIBRATION_FLOOR || iterations == MAX_ITERATIONS {
            let scaled = u128::from(iterations)
                .saturating_mul(TARGET_SAMPLE.as_nanos())
                .div_ceil(elapsed.as_nanos().max(1));
            return u64::try_from(scaled)
                .unwrap_or(MAX_ITERATIONS)
                .clamp(1, MAX_ITERATIONS);
        }
        iterations = iterations.saturating_mul(10).min(MAX_ITERATIONS);
    }
}

fn capture_metadata() -> Vec<(String, String)> {
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).map_or_else(
        |_| "before-unix-epoch".into(),
        |value| value.as_secs().to_string(),
    );
    let commit = command_output("git", &["rev-parse", "HEAD"]);
    let status = command_output("git", &["status", "--porcelain"]);
    let dirty = (!status.is_empty()).to_string();
    let rustc = command_output("rustc", &["-Vv"]);
    let target = rustc
        .split(" | ")
        .find_map(|line| line.strip_prefix("host: "))
        .unwrap_or("unknown")
        .to_owned();
    let cpu = env::var("PROCESSOR_IDENTIFIER")
        .or_else(|_| env::var("HOSTTYPE"))
        .unwrap_or_else(|_| "unknown".into());
    let power_policy = if cfg!(windows) {
        command_output("powercfg", &["/getactivescheme"])
    } else {
        "not captured on this OS".into()
    };
    let os = if cfg!(windows) {
        command_output("cmd", &["/C", "ver"])
    } else {
        command_output("uname", &["-srmo"])
    };
    let profile_overrides = env::vars()
        .filter(|(key, _)| key.starts_with("CARGO_PROFILE_BENCH_"))
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join(",");
    vec![
        ("timestamp_unix_seconds".into(), timestamp),
        ("commit".into(), commit),
        ("dirty".into(), dirty),
        ("rustc_-Vv".into(), rustc),
        ("target".into(), target),
        ("os".into(), os),
        ("cpu".into(), cpu),
        ("power_policy".into(), power_policy),
        (
            "declared_repository_bench_profile".into(),
            "codegen-units=1,lto=thin,overflow-checks=true,panic=unwind (Cargo bench),strip=debuginfo".into(),
        ),
        (
            "cargo_profile_bench_env_overrides".into(),
            if profile_overrides.is_empty() {
                "none observed".into()
            } else {
                profile_overrides
            },
        ),
        ("historical_archive_sha256".into(), HISTORICAL_ARCHIVE_SHA256.into()),
        ("historical_harness_sha256".into(), HISTORICAL_HARNESS_SHA256.into()),
    ]
}

fn command_output(program: &str, arguments: &[&str]) -> String {
    Command::new(program).args(arguments).output().map_or_else(
        |error| format!("unavailable ({error})"),
        |output| {
            let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            if value.is_empty() {
                String::from_utf8_lossy(&output.stderr).trim().to_owned()
            } else {
                value.replace(['\r', '\n'], " | ")
            }
        },
    )
}

const fn suite_name(suite: Suite) -> &'static str {
    match suite {
        Suite::Historical => "historical",
        Suite::Modern => "modern",
        Suite::All => "all",
    }
}

const fn mode_name(mode: Mode) -> &'static str {
    match mode {
        Mode::HistoricalFixed => "historical-fixed",
        Mode::Adaptive => "adaptive",
    }
}

fn csv_row(output: &mut String, fields: &[&str]) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn command_line_options_parse_as_one_explicit_configuration() {
        let config = Config::parse_arguments(arguments(&[
            "--suite",
            "historical",
            "--filter",
            "Fingerprint",
            "--mode",
            "historical-fixed",
            "--format",
            "csv",
            "--output",
            "target/result.csv",
            "--bench",
        ]))
        .unwrap();

        assert_eq!(config.suite, Suite::Historical);
        assert_eq!(config.mode, Mode::HistoricalFixed);
        assert_eq!(config.filter.as_deref(), Some("Fingerprint"));
        assert_eq!(config.format, Format::Csv);
        assert_eq!(config.output, Some(PathBuf::from("target/result.csv")));
        assert!(config.includes(Suite::Historical, "cold fingerprint"));
        assert!(!config.includes(Suite::Modern, "cold fingerprint"));
    }

    #[test]
    fn command_line_rejects_missing_and_invalid_values() {
        assert_eq!(
            Config::parse_arguments(arguments(&["--suite"])).unwrap_err(),
            "--suite requires a value"
        );
        assert!(
            Config::parse_arguments(arguments(&["--mode", "quick"]))
                .unwrap_err()
                .contains("historical-fixed or adaptive")
        );
        assert!(
            Config::parse_arguments(arguments(&["--unknown"]))
                .unwrap_err()
                .contains("unknown benchmark option")
        );
    }

    #[test]
    fn historical_fixed_mode_uses_the_declared_iteration_count_once() {
        let config = Config {
            suite: Suite::Historical,
            mode: Mode::HistoricalFixed,
            filter: Some("fixture".into()),
            format: Format::Text,
            output: None,
        };
        let mut runner = Runner {
            config,
            metadata: Vec::new(),
            measurements: Vec::new(),
            notes: Vec::new(),
            next_order: 0,
        };
        let mut calls = 0_u64;

        runner.measure(Suite::Historical, "fixture", 7, || calls += 1);

        assert_eq!(calls, 7);
        assert_eq!(runner.measurements.len(), 1);
        assert_eq!(runner.measurements[0].iterations, 7);
        assert_eq!(runner.measurements[0].samples, 1);
    }

    #[test]
    fn csv_rows_quote_commas_quotes_and_newlines() {
        let mut output = String::new();
        csv_row(
            &mut output,
            &["plain", "comma,value", "say \"yes\"", "two\nlines"],
        );
        assert_eq!(
            output,
            "\"plain\",\"comma,value\",\"say \"\"yes\"\"\",\"two\nlines\"\n"
        );
    }
}
