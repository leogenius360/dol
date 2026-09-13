//! Dependency-neutral closure-v1 runner used inside frozen revision worktrees.
//!
//! This target intentionally depends only on the DOL crates already present in
//! both original `Cargo.lock` files. Statistical analysis and final JSON
//! composition remain in the current orchestrator; this process emits ordered
//! raw observations from the identical workload overlay.

#[allow(dead_code, unused_imports)]
#[path = "../src/workloads.rs"]
mod workloads;

fn main() {
    let config = canonical::CanonicalConfig::parse_env().unwrap_or_else(|error| {
        eprintln!("canonical overlay argument error: {error}");
        std::process::exit(2);
    });
    let mut runner = canonical::CanonicalRunner::new(config);
    workloads::run_closure_v1(&mut runner).unwrap_or_else(|error| {
        eprintln!("canonical overlay setup failed: {error}");
        std::process::exit(1);
    });
    runner.finish_and_emit().unwrap_or_else(|error| {
        eprintln!("canonical overlay output failed: {error}");
        std::process::exit(1);
    });
}

mod scenario {
    use std::fmt;

    #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
    pub enum Lifecycle {
        Cold,
        Warm,
        Prepared,
        Fresh,
        SharedPlan,
    }

    impl Lifecycle {
        pub const fn as_str(self) -> &'static str {
            match self {
                Self::Cold => "cold",
                Self::Warm => "warm",
                Self::Prepared => "prepared",
                Self::Fresh => "fresh",
                Self::SharedPlan => "shared-plan",
            }
        }
    }

    impl fmt::Display for Lifecycle {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str(self.as_str())
        }
    }

    #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
    pub enum WorkUnitKind {
        Operation,
        Evaluation,
        Node,
        Stage,
        Field,
        Byte,
        Row,
        Candidate,
    }

    impl WorkUnitKind {
        pub const fn as_str(self) -> &'static str {
            match self {
                Self::Operation => "operation",
                Self::Evaluation => "evaluation",
                Self::Node => "node",
                Self::Stage => "stage",
                Self::Field => "field",
                Self::Byte => "byte",
                Self::Row => "row",
                Self::Candidate => "candidate",
            }
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct ScenarioDefinition {
        pub id: &'static str,
        pub lifecycle: Lifecycle,
        pub work_unit_kind: WorkUnitKind,
    }

    pub fn scenario_definition(
        id: &'static str,
        lifecycle: Lifecycle,
    ) -> Option<ScenarioDefinition> {
        let valid = !id.is_empty()
            && !id.starts_with('.')
            && !id.ends_with('.')
            && !id.contains("..")
            && id.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-')
            });
        if !valid {
            return None;
        }
        let work_unit_kind = if id.starts_with("expr.eval.depth.") {
            WorkUnitKind::Node
        } else if id.starts_with("pipeline.lower.stages.") {
            WorkUnitKind::Stage
        } else if id.starts_with("model.define") {
            WorkUnitKind::Field
        } else if id.starts_with("wire.decode") {
            WorkUnitKind::Byte
        } else if id.starts_with("memory.execute") {
            WorkUnitKind::Row
        } else if id == "vector.search.exact" {
            WorkUnitKind::Candidate
        } else if id == "expr.eval.prepared" {
            WorkUnitKind::Evaluation
        } else {
            WorkUnitKind::Operation
        };
        Some(ScenarioDefinition {
            id,
            lifecycle,
            work_unit_kind,
        })
    }
}

mod canonical {
    use std::env;
    use std::fs;
    use std::hint::black_box;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    use crate::scenario::ScenarioDefinition;

    const CALIBRATION_FLOOR: Duration = Duration::from_millis(10);
    const MAX_ITERATIONS: u64 = 50_000_000;

    #[derive(Clone, Copy)]
    enum Profile {
        HistoricalFixed,
        Adaptive,
        Smoke,
        Candidate,
        Release,
    }

    impl Profile {
        fn parse(value: &str) -> Result<Self, String> {
            match value {
                "historical-fixed" => Ok(Self::HistoricalFixed),
                "adaptive" => Ok(Self::Adaptive),
                "smoke" => Ok(Self::Smoke),
                "candidate" => Ok(Self::Candidate),
                "release" => Ok(Self::Release),
                other => Err(format!("unknown closure-v1 profile `{other}`")),
            }
        }

        const fn configuration(self) -> Sampling {
            match self {
                Self::HistoricalFixed => Sampling::new(0, 1, 0, true),
                Self::Adaptive => Sampling::new(1_000, 9, 150, false),
                Self::Smoke => Sampling::new(250, 5, 50, false),
                Self::Candidate => Sampling::new(3_000, 20, 300, false),
                Self::Release => Sampling::new(3_000, 30, 500, false),
            }
        }
    }

    #[derive(Clone, Copy)]
    struct Sampling {
        warmup_ms: u64,
        samples: u32,
        target_sample_ms: u64,
        fixed_iterations: bool,
    }

    impl Sampling {
        const fn new(
            warmup_ms: u64,
            samples: u32,
            target_sample_ms: u64,
            fixed_iterations: bool,
        ) -> Self {
            Self {
                warmup_ms,
                samples,
                target_sample_ms,
                fixed_iterations,
            }
        }
    }

    #[derive(Clone, Copy)]
    enum Side {
        Baseline,
        Candidate,
    }

    impl Side {
        const fn as_str(self) -> &'static str {
            match self {
                Self::Baseline => "baseline",
                Self::Candidate => "candidate",
            }
        }
    }

    #[derive(Clone, Copy)]
    enum PairOrder {
        BaselineFirst,
        CandidateFirst,
    }

    impl PairOrder {
        const fn as_str(self) -> &'static str {
            match self {
                Self::BaselineFirst => "baseline-first",
                Self::CandidateFirst => "candidate-first",
            }
        }
    }

    pub struct CanonicalConfig {
        profile: Profile,
        artifact_dir: Option<PathBuf>,
        scenario_filter: Option<String>,
        side: Side,
        pair: u32,
        attempt: u32,
        pair_order: PairOrder,
        order_start: u64,
    }

    impl CanonicalConfig {
        pub fn parse_env() -> Result<Self, String> {
            let mut config = Self {
                profile: Profile::Adaptive,
                artifact_dir: None,
                scenario_filter: None,
                side: Side::Candidate,
                pair: 0,
                attempt: 0,
                pair_order: PairOrder::BaselineFirst,
                order_start: 0,
            };
            let mut arguments = env::args().skip(1);
            while let Some(argument) = arguments.next() {
                match argument.as_str() {
                    "--profile" => {
                        config.profile = Profile::parse(&next(&mut arguments, &argument)?)?
                    }
                    "--artifact-dir" => {
                        config.artifact_dir = Some(PathBuf::from(next(&mut arguments, &argument)?));
                    }
                    "--scenario" => config.scenario_filter = Some(next(&mut arguments, &argument)?),
                    "--side" => {
                        config.side = match next(&mut arguments, &argument)?.as_str() {
                            "baseline" => Side::Baseline,
                            "candidate" | "single" => Side::Candidate,
                            other => return Err(format!("unknown revision side `{other}`")),
                        };
                    }
                    "--pair" => config.pair = number(&mut arguments, &argument)?,
                    "--attempt" => config.attempt = number(&mut arguments, &argument)?,
                    "--pair-order" => {
                        config.pair_order = match next(&mut arguments, &argument)?.as_str() {
                            "baseline-first" => PairOrder::BaselineFirst,
                            "candidate-first" => PairOrder::CandidateFirst,
                            other => return Err(format!("unknown pair order `{other}`")),
                        };
                    }
                    "--order" | "--order-start" => {
                        config.order_start = number(&mut arguments, &argument)?;
                    }
                    "--source-sha" | "--fixture-hash" | "--lockfile-hash" | "--runner" => {
                        let _ = next(&mut arguments, &argument)?;
                    }
                    "--bench" => {}
                    other => return Err(format!("unknown canonical overlay option `{other}`")),
                }
            }
            if config.attempt > 1 {
                return Err("closure-v1 permits only attempt 0 or 1".into());
            }
            Ok(config)
        }

        fn includes(&self, scenario: ScenarioDefinition) -> bool {
            self.scenario_filter
                .as_ref()
                .is_none_or(|filter| scenario.id == filter)
        }
    }

    struct RawSample {
        pair: u32,
        attempt: u32,
        side: Side,
        pair_order: PairOrder,
        order: u64,
        scenario: ScenarioDefinition,
        sample: u32,
        iterations: u64,
        elapsed_ns: u64,
        ns_per_op: f64,
        throughput: f64,
        work_units: u64,
    }

    pub struct CanonicalRunner {
        config: CanonicalConfig,
        sampling: Sampling,
        samples: Vec<RawSample>,
        next_order: u64,
    }

    impl CanonicalRunner {
        pub fn new(config: CanonicalConfig) -> Self {
            Self {
                sampling: config.profile.configuration(),
                next_order: config.order_start,
                config,
                samples: Vec::new(),
            }
        }

        pub fn enabled(&self, scenario: ScenarioDefinition) -> bool {
            self.config.includes(scenario)
        }

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
                warm_up(
                    &mut operation,
                    Duration::from_millis(self.sampling.warmup_ms),
                );
            }
            let iterations = if self.sampling.fixed_iterations {
                historical_iterations.max(1)
            } else {
                calibrate(
                    &mut operation,
                    Duration::from_millis(self.sampling.target_sample_ms),
                )
            };
            for sample in 0..self.sampling.samples {
                let started = Instant::now();
                for _ in 0..iterations {
                    black_box(operation());
                }
                let elapsed_ns = u64::try_from(started.elapsed().as_nanos())
                    .unwrap_or(u64::MAX)
                    .max(1);
                let ns_per_op = elapsed_ns as f64 / iterations as f64;
                let throughput = work_units as f64 * 1_000_000_000.0 / ns_per_op;
                self.samples.push(RawSample {
                    pair: self.config.pair,
                    attempt: self.config.attempt,
                    side: self.config.side,
                    pair_order: self.config.pair_order,
                    order: self.next_order,
                    scenario,
                    sample,
                    iterations,
                    elapsed_ns,
                    ns_per_op,
                    throughput,
                    work_units,
                });
                self.next_order = self
                    .next_order
                    .checked_add(1)
                    .ok_or_else(|| "raw observation order overflow".to_string())?;
            }
            Ok(())
        }

        pub fn finish_and_emit(self) -> Result<(), String> {
            if self.samples.is_empty() {
                return Err("no canonical scenarios matched the requested filter".into());
            }
            let requested = self
                .config
                .artifact_dir
                .ok_or_else(|| "overlay runner requires --artifact-dir".to_string())?;
            let directory = if requested.is_absolute() {
                requested
            } else {
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../..")
                    .join(requested)
            };
            fs::create_dir_all(&directory)
                .map_err(|error| format!("cannot create {}: {error}", directory.display()))?;
            let mut output = String::from(
                "\"pair\",\"attempt\",\"side\",\"pair_order\",\"order\",\"scenario_id\",\"lifecycle\",\"sample\",\"iterations\",\"elapsed_ns\",\"ns_per_op\",\"throughput\",\"work_units\",\"work_unit_kind\",\"tukey_class\",\"valid_for_comparison\",\"rejection_reason\"\n",
            );
            for sample in self.samples {
                csv_record(
                    &mut output,
                    &[
                        sample.pair.to_string(),
                        sample.attempt.to_string(),
                        sample.side.as_str().into(),
                        sample.pair_order.as_str().into(),
                        sample.order.to_string(),
                        sample.scenario.id.into(),
                        sample.scenario.lifecycle.as_str().into(),
                        sample.sample.to_string(),
                        sample.iterations.to_string(),
                        sample.elapsed_ns.to_string(),
                        sample.ns_per_op.to_string(),
                        sample.throughput.to_string(),
                        sample.work_units.to_string(),
                        sample.scenario.work_unit_kind.as_str().into(),
                        "inlier".into(),
                        "true".into(),
                        String::new(),
                    ],
                );
            }
            let path = directory.join("raw-samples.csv");
            fs::write(&path, output)
                .map_err(|error| format!("cannot write {}: {error}", path.display()))
        }
    }

    fn warm_up<T>(operation: &mut impl FnMut() -> T, duration: Duration) {
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

    fn next(arguments: &mut impl Iterator<Item = String>, option: &str) -> Result<String, String> {
        arguments
            .next()
            .ok_or_else(|| format!("{option} requires a value"))
    }

    fn number<T>(arguments: &mut impl Iterator<Item = String>, option: &str) -> Result<T, String>
    where
        T: std::str::FromStr,
        T::Err: std::fmt::Display,
    {
        let value = next(arguments, option)?;
        value
            .parse()
            .map_err(|error| format!("invalid {option} value `{value}`: {error}"))
    }
}
