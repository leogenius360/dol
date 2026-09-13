use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use dol_bench::sample::PairOrder;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Side {
    Baseline,
    Candidate,
}

impl Side {
    const fn name(self) -> &'static str {
        match self {
            Self::Baseline => "baseline",
            Self::Candidate => "candidate",
        }
    }
}

pub(super) struct CanonicalRun<'a> {
    pub(super) worktree: &'a Path,
    pub(super) target_dir: &'a Path,
    pub(super) profile: &'a str,
    pub(super) artifact_dir: &'a Path,
    pub(super) side: Side,
    pub(super) pair: u32,
    pub(super) pair_order: PairOrder,
    pub(super) order: u64,
    pub(super) attempt: u32,
    pub(super) source_sha: &'a str,
    pub(super) scenario: Option<&'a str>,
    pub(super) cpu_list: Option<&'a str>,
}

impl CanonicalRun<'_> {
    fn arguments(&self) -> Vec<String> {
        let mut arguments = vec![
            "bench".to_owned(),
            "--locked".to_owned(),
            "--package".to_owned(),
            "dol-bench".to_owned(),
            "--bench".to_owned(),
            "closure_v1_overlay".to_owned(),
            "--".to_owned(),
            "--profile".to_owned(),
            self.profile.to_owned(),
            "--artifact-dir".to_owned(),
            self.artifact_dir.display().to_string(),
            "--side".to_owned(),
            self.side.name().to_owned(),
            "--pair".to_owned(),
            self.pair.to_string(),
            "--pair-order".to_owned(),
            self.pair_order.as_str().to_owned(),
            "--order".to_owned(),
            self.order.to_string(),
            "--attempt".to_owned(),
            self.attempt.to_string(),
            "--source-sha".to_owned(),
            self.source_sha.to_owned(),
        ];
        if let Some(scenario) = self.scenario {
            arguments.push("--scenario".to_owned());
            arguments.push(scenario.to_owned());
        }
        arguments
    }

    pub(super) fn execute(&self) -> Result<(), String> {
        create_empty_directory(self.artifact_dir)?;
        let arguments = self.arguments();

        let mut command = if let Some(cpu_list) = self.cpu_list {
            let mut command = Command::new("taskset");
            command
                .args(["--cpu-list", cpu_list, "cargo"])
                .args(&arguments);
            command
        } else {
            let mut command = Command::new("cargo");
            command.args(&arguments);
            command
        };
        command
            .current_dir(self.worktree)
            .env("CARGO_TARGET_DIR", self.target_dir)
            .stdout(Stdio::null());
        let status = command.status().map_err(|error| {
            format!(
                "cannot run canonical {} side for pair {} attempt {}: {error}",
                self.side.name(),
                self.pair,
                self.attempt
            )
        })?;
        if !status.success() {
            return Err(format!(
                "canonical {} side for pair {} attempt {} exited with {status}",
                self.side.name(),
                self.pair,
                self.attempt
            ));
        }
        let path = self.artifact_dir.join("raw-samples.csv");
        if !path.is_file() {
            return Err(format!(
                "canonical benchmark did not create required artifact {}",
                path.display()
            ));
        }
        Ok(())
    }
}

pub(super) fn create_empty_directory(path: &Path) -> Result<(), String> {
    if path.exists() {
        if !path.is_dir() {
            return Err(format!(
                "artifact path is not a directory: {}",
                path.display()
            ));
        }
        let mut entries = fs::read_dir(path)
            .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
        if entries
            .next()
            .transpose()
            .map_err(|error| {
                format!(
                    "cannot inspect artifact directory {}: {error}",
                    path.display()
                )
            })?
            .is_some()
        {
            return Err(format!(
                "artifact directory must be absent or empty: {}",
                path.display()
            ));
        }
    } else {
        fs::create_dir_all(path)
            .map_err(|error| format!("cannot create {}: {error}", path.display()))?;
    }
    Ok(())
}

pub(super) fn absolute_output_path(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        Ok(path.to_owned())
    } else {
        std::env::current_dir()
            .map(|directory| directory.join(path))
            .map_err(|error| format!("cannot resolve current directory: {error}"))
    }
}

pub(super) fn reject_profile_overrides() -> Result<(), String> {
    let overrides = std::env::vars_os()
        .filter_map(|(key, _)| key.into_string().ok())
        .filter(|key| key.starts_with("CARGO_PROFILE_BENCH_"))
        .collect::<Vec<_>>();
    if overrides.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "canonical comparison refuses Cargo bench profile overrides: {}",
            overrides.join(", ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn side_names_are_stable_schema_values() {
        assert_eq!(Side::Baseline.name(), "baseline");
        assert_eq!(Side::Candidate.name(), "candidate");
    }

    #[test]
    fn canonical_arguments_preserve_pair_order_and_scenario() {
        let run = CanonicalRun {
            worktree: Path::new("worktree"),
            target_dir: Path::new("target"),
            profile: "candidate",
            artifact_dir: Path::new("artifacts"),
            side: Side::Candidate,
            pair: 7,
            pair_order: PairOrder::CandidateFirst,
            order: 11,
            attempt: 1,
            source_sha: "0123456789abcdef",
            scenario: Some("expr.eval.prepared"),
            cpu_list: None,
        };
        let arguments = run.arguments();
        assert!(
            arguments.windows(2).any(|values| {
                values == ["--pair-order".to_owned(), "candidate-first".to_owned()]
            })
        );
        assert!(arguments.windows(2).any(|values| {
            values == ["--scenario".to_owned(), "expr.eval.prepared".to_owned()]
        }));
    }
}
