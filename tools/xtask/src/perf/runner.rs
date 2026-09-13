use dol_bench::artifact::Metadata;
use serde::Deserialize;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const SCHEMA: &str = "dol-perf-runner/v1";
const TARGET: &str = "x86_64-unknown-linux-gnu";
const RUST_VERSION: &str = "1.98.0";

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RunnerManifest {
    pub(super) schema: String,
    pub(super) runner_id: String,
    pub(super) qualified: bool,
    pub(super) arch: String,
    pub(super) os: String,
    pub(super) target: String,
    pub(super) toolchain: Toolchain,
    pub(super) cpu: Cpu,
    pub(super) affinity: Affinity,
    pub(super) isolation: Isolation,
    pub(super) qualification: Qualification,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Toolchain {
    rust: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Cpu {
    model: String,
    physical_cpu: usize,
    smt: bool,
    boost: bool,
    governor: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Affinity {
    pub(super) cpus: Vec<usize>,
    numa_node: usize,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Isolation {
    irq_policy: String,
    concurrent_jobs: usize,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Qualification {
    minimum_aa_comparisons: usize,
    observed_aa_comparisons: usize,
    minimum_calendar_days: usize,
    observed_calendar_days: usize,
    false_hard_regressions: usize,
    known_ab_stable: bool,
}

impl RunnerManifest {
    pub(super) fn read(path: &Path) -> Result<Self, String> {
        let content = fs::read_to_string(path)
            .map_err(|error| format!("cannot read runner manifest {}: {error}", path.display()))?;
        let manifest: Self = toml::from_str(&content)
            .map_err(|error| format!("invalid runner manifest {}: {error}", path.display()))?;
        manifest.validate_shape()?;
        Ok(manifest)
    }

    fn validate_shape(&self) -> Result<(), String> {
        expect(&self.schema, SCHEMA, "schema")?;
        expect(&self.arch, "x86_64", "arch")?;
        expect(&self.os, "linux", "os")?;
        expect(&self.target, TARGET, "target")?;
        expect(&self.toolchain.rust, RUST_VERSION, "toolchain.rust")?;
        non_placeholder(&self.runner_id, "runner_id")?;
        non_placeholder(&self.cpu.model, "cpu.model")?;
        non_placeholder(&self.isolation.irq_policy, "isolation.irq_policy")?;
        if !self.qualified {
            return Err("runner manifest is not qualified; canonical comparison refused".into());
        }
        if self.cpu.smt {
            return Err("runner manifest must require cpu.smt=false".into());
        }
        if self.cpu.boost {
            return Err("runner manifest must require cpu.boost=false".into());
        }
        expect(&self.cpu.governor, "performance", "cpu.governor")?;
        if self.affinity.cpus.is_empty() {
            return Err("runner manifest affinity.cpus cannot be empty".into());
        }
        let unique = self.affinity.cpus.iter().copied().collect::<HashSet<_>>();
        if unique.len() != self.affinity.cpus.len() {
            return Err("runner manifest affinity.cpus contains duplicates".into());
        }
        if !unique.contains(&self.cpu.physical_cpu) {
            return Err("cpu.physical_cpu must be included in affinity.cpus".into());
        }
        if self.isolation.concurrent_jobs != 1 {
            return Err("runner manifest isolation.concurrent_jobs must equal 1".into());
        }
        let qualification = &self.qualification;
        if qualification.minimum_aa_comparisons < 20
            || qualification.observed_aa_comparisons < qualification.minimum_aa_comparisons
        {
            return Err("runner has not completed at least 20 A/A comparisons".into());
        }
        if qualification.minimum_calendar_days < 14
            || qualification.observed_calendar_days < qualification.minimum_calendar_days
        {
            return Err("runner has not completed at least 14 qualification days".into());
        }
        if qualification.false_hard_regressions != 0 {
            return Err("runner qualification contains a false hard regression".into());
        }
        if !qualification.known_ab_stable {
            return Err("runner has not demonstrated stable known A/B signal".into());
        }
        Ok(())
    }

    pub(super) fn validate_live(&self) -> Result<(), String> {
        if !cfg!(target_os = "linux") {
            return Err(
                "candidate/release performance comparison requires a qualified Linux runner".into(),
            );
        }
        expect(&output("uname", &["-s"])?, "Linux", "live operating system")?;
        expect(&output("uname", &["-m"])?, "x86_64", "live architecture")?;
        let rustc = output("rustc", &["--version"])?;
        if !rustc.starts_with(&format!("rustc {RUST_VERSION} ")) {
            return Err(format!(
                "runner invalid: expected rustc {RUST_VERSION}, observed `{rustc}`"
            ));
        }
        let verbose_rustc = output("rustc", &["-Vv"])?;
        let host = field(&verbose_rustc, "host:")
            .ok_or_else(|| "rustc -Vv did not report a host target".to_string())?;
        expect(host, TARGET, "live rustc target")?;

        let allowed = fs::read_to_string("/proc/self/status")
            .map_err(|error| format!("cannot read live CPU affinity: {error}"))?;
        let allowed = field(&allowed, "Cpus_allowed_list:")
            .ok_or_else(|| "/proc/self/status has no Cpus_allowed_list".to_string())?;
        let allowed = parse_cpu_list(allowed)?;
        for cpu in &self.affinity.cpus {
            if !allowed.contains(cpu) {
                return Err(format!(
                    "runner invalid: affinity CPU {cpu} is unavailable (allowed: {allowed:?})"
                ));
            }
        }

        let cpuinfo = fs::read_to_string("/proc/cpuinfo")
            .map_err(|error| format!("cannot read /proc/cpuinfo: {error}"))?;
        let model = cpu_model(&cpuinfo, self.cpu.physical_cpu).ok_or_else(|| {
            format!(
                "runner invalid: CPU {} is missing from /proc/cpuinfo",
                self.cpu.physical_cpu
            )
        })?;
        expect(model, &self.cpu.model, "live CPU model")?;

        expect_bool_file("/sys/devices/system/cpu/smt/active", self.cpu.smt, false)?;
        validate_boost(self.cpu.boost)?;
        for cpu in &self.affinity.cpus {
            let governor = fs::read_to_string(format!(
                "/sys/devices/system/cpu/cpu{cpu}/cpufreq/scaling_governor"
            ))
            .map_err(|error| format!("cannot read governor for CPU {cpu}: {error}"))?;
            expect(governor.trim(), &self.cpu.governor, "live CPU governor")?;
            let numa = PathBuf::from(format!(
                "/sys/devices/system/cpu/cpu{cpu}/node{}",
                self.affinity.numa_node
            ));
            if !numa.exists() {
                return Err(format!(
                    "runner invalid: CPU {cpu} is not on NUMA node {}",
                    self.affinity.numa_node
                ));
            }
        }
        require_program("taskset", &["--version"])?;
        Ok(())
    }

    pub(super) fn cpu_list(&self) -> String {
        self.affinity
            .cpus
            .iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join(",")
    }

    pub(super) fn annotate_metadata(&self, metadata: &mut Metadata) {
        let cpu_list = self.cpu_list();
        metadata.cpu_topology = format!(
            "model={}; physical_cpu={}; affinity_cpus={cpu_list}",
            self.cpu.model, self.cpu.physical_cpu
        );
        metadata.affinity = cpu_list;
        metadata.smt = self.cpu.smt.to_string();
        metadata.governor.clone_from(&self.cpu.governor);
        metadata.boost = self.cpu.boost.to_string();
        metadata.numa = format!("node {}", self.affinity.numa_node);
        metadata
            .environment_identity
            .insert("irq_policy".into(), self.isolation.irq_policy.clone());
        metadata.environment_identity.insert(
            "concurrent_jobs".into(),
            self.isolation.concurrent_jobs.to_string(),
        );
        metadata
            .environment_identity
            .insert("runner_qualified".into(), self.qualified.to_string());
        if metadata.microcode.is_none()
            && let Ok(cpuinfo) = fs::read_to_string("/proc/cpuinfo")
        {
            metadata.microcode = cpuinfo_section(&cpuinfo, self.cpu.physical_cpu)
                .and_then(|section| cpuinfo_value(section, "microcode"))
                .map(str::to_owned);
        }
    }
}

fn expect(actual: &str, expected: &str, field_name: &str) -> Result<(), String> {
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "runner invalid: expected {field_name}={expected}, observed `{actual}`"
        ))
    }
}

fn non_placeholder(value: &str, field_name: &str) -> Result<(), String> {
    if value.trim().is_empty() || value.eq_ignore_ascii_case("UNPROVISIONED") {
        Err(format!("runner manifest {field_name} is not provisioned"))
    } else {
        Ok(())
    }
}

fn output(program: &str, arguments: &[&str]) -> Result<String, String> {
    let output = Command::new(program)
        .args(arguments)
        .output()
        .map_err(|error| format!("cannot run `{program}`: {error}"))?;
    if !output.status.success() {
        return Err(format!("`{program}` exited with {}", output.status));
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_owned())
        .map_err(|_| format!("`{program}` returned non-UTF-8 output"))
}

fn require_program(program: &str, arguments: &[&str]) -> Result<(), String> {
    match Command::new(program)
        .args(arguments)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(format!("required program `{program}` exited with {status}")),
        Err(error) => Err(format!(
            "required program `{program}` is unavailable: {error}"
        )),
    }
}

fn field<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    text.lines()
        .find_map(|line| line.trim().strip_prefix(prefix).map(str::trim))
}

fn parse_cpu_list(value: &str) -> Result<HashSet<usize>, String> {
    let mut cpus = HashSet::new();
    for range in value.split(',') {
        let mut bounds = range.trim().split('-');
        let first = bounds
            .next()
            .and_then(|value| value.parse::<usize>().ok())
            .ok_or_else(|| format!("invalid Linux CPU list `{value}`"))?;
        let last = bounds
            .next()
            .map_or(Ok(first), |value| value.parse::<usize>())
            .map_err(|_| format!("invalid Linux CPU list `{value}`"))?;
        if bounds.next().is_some() || first > last {
            return Err(format!("invalid Linux CPU list `{value}`"));
        }
        cpus.extend(first..=last);
    }
    Ok(cpus)
}

fn cpu_model(cpuinfo: &str, selected_cpu: usize) -> Option<&str> {
    cpuinfo_section(cpuinfo, selected_cpu).and_then(|section| cpuinfo_value(section, "model name"))
}

fn cpuinfo_section(cpuinfo: &str, selected_cpu: usize) -> Option<&str> {
    cpuinfo.split("\n\n").find(|section| {
        cpuinfo_value(section, "processor").and_then(|value| value.parse::<usize>().ok())
            == Some(selected_cpu)
    })
}

fn cpuinfo_value<'a>(section: &'a str, key: &str) -> Option<&'a str> {
    section.lines().find_map(|line| {
        let (observed_key, value) = line.split_once(':')?;
        (observed_key.trim() == key).then_some(value.trim())
    })
}

fn expect_bool_file(path: &str, expected: bool, invert: bool) -> Result<(), String> {
    let value =
        fs::read_to_string(path).map_err(|error| format!("cannot inspect {path}: {error}"))?;
    let mut observed = match value.trim() {
        "0" => false,
        "1" => true,
        other => return Err(format!("unexpected boolean `{other}` in {path}")),
    };
    if invert {
        observed = !observed;
    }
    if observed == expected {
        Ok(())
    } else {
        Err(format!(
            "runner invalid: expected {}={expected}, observed {observed}",
            Path::new(path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(path)
        ))
    }
}

fn validate_boost(expected: bool) -> Result<(), String> {
    let intel = "/sys/devices/system/cpu/intel_pstate/no_turbo";
    let generic = "/sys/devices/system/cpu/cpufreq/boost";
    if Path::new(intel).is_file() {
        return expect_bool_file(intel, expected, true);
    }
    if Path::new(generic).is_file() {
        return expect_bool_file(generic, expected, false);
    }
    Err("cannot validate CPU boost state: no supported sysfs control exists".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_cpu_lists_are_parsed_strictly() {
        assert_eq!(
            parse_cpu_list("0-2,7").unwrap(),
            HashSet::from([0, 1, 2, 7])
        );
        assert!(parse_cpu_list("4-2").is_err());
        assert!(parse_cpu_list("one").is_err());
    }

    #[test]
    fn cpu_model_is_selected_by_processor_number() {
        let text = "processor : 2\nmodel name : Other\n\nprocessor : 6\nmodel name : Expected\n";
        assert_eq!(cpu_model(text, 6), Some("Expected"));
    }

    #[test]
    fn unqualified_manifest_is_rejected_before_live_checks() {
        let source = r#"
schema = "dol-perf-runner/v1"
runner_id = "test-runner"
qualified = false
arch = "x86_64"
os = "linux"
target = "x86_64-unknown-linux-gnu"
[toolchain]
rust = "1.98.0"
[cpu]
model = "Example CPU"
physical_cpu = 6
smt = false
boost = false
governor = "performance"
[affinity]
cpus = [6]
numa_node = 0
[isolation]
irq_policy = "isolcpus"
concurrent_jobs = 1
[qualification]
minimum_aa_comparisons = 20
observed_aa_comparisons = 20
minimum_calendar_days = 14
observed_calendar_days = 14
false_hard_regressions = 0
known_ab_stable = true
"#;
        let manifest: RunnerManifest = toml::from_str(source).unwrap();
        assert!(
            manifest
                .validate_shape()
                .unwrap_err()
                .contains("not qualified")
        );
    }
}
