use std::collections::HashSet;
use std::path::PathBuf;

pub(super) const CONTRACT: &str = "closure-v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ComparisonProfile {
    Candidate,
    Release,
}

impl ComparisonProfile {
    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::Candidate => "candidate",
            Self::Release => "release",
        }
    }

    pub(super) const fn pairs(self) -> u32 {
        match self {
            Self::Candidate => 7,
            Self::Release => 10,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CompareArgs {
    pub(super) baseline: String,
    pub(super) candidate: String,
    pub(super) profile: ComparisonProfile,
    pub(super) runner_manifest: PathBuf,
    pub(super) artifact_dir: PathBuf,
    pub(super) enforce: bool,
}

impl CompareArgs {
    pub(super) fn parse(arguments: &[String]) -> Result<Self, String> {
        let mut parser = Options::new(arguments);
        let mut baseline = None;
        let mut candidate = None;
        let mut profile = None;
        let mut contract = None;
        let mut runner_manifest = None;
        let mut artifact_dir = None;
        let mut enforce = false;

        while let Some(option) = parser.next_option()? {
            match option.as_str() {
                "--baseline" => baseline = Some(parser.value(&option)?),
                "--candidate" => candidate = Some(parser.value(&option)?),
                "--profile" => {
                    profile = Some(match parser.value(&option)?.as_str() {
                        "candidate" => ComparisonProfile::Candidate,
                        "release" => ComparisonProfile::Release,
                        other => {
                            return Err(format!(
                                "invalid --profile `{other}`; expected candidate or release"
                            ));
                        }
                    });
                }
                "--contract" => contract = Some(parser.value(&option)?),
                "--runner-manifest" => {
                    runner_manifest = Some(PathBuf::from(parser.value(&option)?));
                }
                "--artifact-dir" => artifact_dir = Some(PathBuf::from(parser.value(&option)?)),
                "--enforce" => enforce = true,
                _ => return Err(unknown("perf-compare", &option)),
            }
        }

        let contract = required(contract, "--contract")?;
        if contract != CONTRACT {
            return Err(format!(
                "unsupported performance contract `{contract}`; expected {CONTRACT}"
            ));
        }
        Ok(Self {
            baseline: required(baseline, "--baseline")?,
            candidate: required(candidate, "--candidate")?,
            profile: required(profile, "--profile")?,
            runner_manifest: required(runner_manifest, "--runner-manifest")?,
            artifact_dir: required(artifact_dir, "--artifact-dir")?,
            enforce,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SmokeArgs {
    pub(super) artifact_dir: PathBuf,
}

impl SmokeArgs {
    pub(super) fn parse(arguments: &[String]) -> Result<Self, String> {
        let mut parser = Options::new(arguments);
        let mut artifact_dir = None;
        while let Some(option) = parser.next_option()? {
            match option.as_str() {
                "--artifact-dir" => artifact_dir = Some(PathBuf::from(parser.value(&option)?)),
                _ => return Err(unknown("perf-a-a-smoke", &option)),
            }
        }
        Ok(Self {
            artifact_dir: required(artifact_dir, "--artifact-dir")?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ProfileArgs {
    pub(super) commit: String,
    pub(super) scenario: String,
    pub(super) artifact_dir: PathBuf,
}

impl ProfileArgs {
    pub(super) fn parse(arguments: &[String]) -> Result<Self, String> {
        let mut parser = Options::new(arguments);
        let mut commit = None;
        let mut scenario = None;
        let mut artifact_dir = None;
        while let Some(option) = parser.next_option()? {
            match option.as_str() {
                "--commit" => commit = Some(parser.value(&option)?),
                "--scenario" => scenario = Some(parser.value(&option)?),
                "--artifact-dir" => artifact_dir = Some(PathBuf::from(parser.value(&option)?)),
                _ => return Err(unknown("perf-profile", &option)),
            }
        }
        let scenario = required(scenario, "--scenario")?;
        validate_scenario_id(&scenario)?;
        Ok(Self {
            commit: required(commit, "--commit")?,
            scenario,
            artifact_dir: required(artifact_dir, "--artifact-dir")?,
        })
    }
}

struct Options<'a> {
    arguments: &'a [String],
    position: usize,
    seen: HashSet<String>,
}

impl<'a> Options<'a> {
    fn new(arguments: &'a [String]) -> Self {
        Self {
            arguments,
            position: 0,
            seen: HashSet::new(),
        }
    }

    fn next_option(&mut self) -> Result<Option<String>, String> {
        let Some(argument) = self.arguments.get(self.position) else {
            return Ok(None);
        };
        self.position += 1;
        if !argument.starts_with("--") || argument == "--" || argument.contains('=') {
            return Err(format!(
                "unexpected argument `{argument}`; options must use `--name value` syntax"
            ));
        }
        if !self.seen.insert(argument.clone()) {
            return Err(format!("duplicate option `{argument}`"));
        }
        Ok(Some(argument.clone()))
    }

    fn value(&mut self, option: &str) -> Result<String, String> {
        let value = self
            .arguments
            .get(self.position)
            .ok_or_else(|| format!("{option} requires a value"))?;
        if value.starts_with("--") {
            return Err(format!("{option} requires a value"));
        }
        self.position += 1;
        if value.is_empty() {
            return Err(format!("{option} cannot be empty"));
        }
        Ok(value.clone())
    }
}

fn required<T>(value: Option<T>, option: &str) -> Result<T, String> {
    value.ok_or_else(|| format!("missing required option {option}"))
}

fn unknown(command: &str, option: &str) -> String {
    format!("unknown {command} option `{option}`")
}

fn validate_scenario_id(value: &str) -> Result<(), String> {
    let bytes = value.as_bytes();
    if bytes.is_empty()
        || !bytes[0].is_ascii_lowercase()
        || bytes.iter().any(|byte| {
            !(byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-'))
        })
        || value.contains("..")
        || value.ends_with(['.', '-'])
    {
        return Err(format!(
            "invalid scenario ID `{value}`; expected a stable lowercase dotted identifier"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn comparison_parser_requires_the_complete_contract() {
        let parsed = CompareArgs::parse(&args(&[
            "--baseline",
            "base",
            "--candidate",
            "head",
            "--profile",
            "candidate",
            "--contract",
            "closure-v1",
            "--runner-manifest",
            "runner.toml",
            "--artifact-dir",
            "out",
            "--enforce",
        ]))
        .unwrap();
        assert_eq!(parsed.profile, ComparisonProfile::Candidate);
        assert!(parsed.enforce);
        assert_eq!(parsed.runner_manifest, PathBuf::from("runner.toml"));
    }

    #[test]
    fn comparison_parser_rejects_unknown_duplicate_and_noncanonical_values() {
        assert!(CompareArgs::parse(&args(&["--baseline", "a"])).is_err());
        assert!(
            CompareArgs::parse(&args(&["--baseline", "a", "--baseline", "b"]))
                .unwrap_err()
                .contains("duplicate")
        );
        assert!(
            CompareArgs::parse(&args(&["--contract", "closure-v2"]))
                .unwrap_err()
                .contains("unsupported")
        );
        assert!(
            CompareArgs::parse(&args(&["--profile=release"]))
                .unwrap_err()
                .contains("--name value")
        );
    }

    #[test]
    fn profile_parser_rejects_path_like_scenario_ids() {
        assert!(
            ProfileArgs::parse(&args(&[
                "--commit",
                "HEAD",
                "--scenario",
                "../../wire",
                "--artifact-dir",
                "out",
            ]))
            .unwrap_err()
            .contains("scenario ID")
        );
    }
}
