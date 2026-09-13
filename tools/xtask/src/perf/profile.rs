use super::cli::ProfileArgs;
use super::execute::{absolute_output_path, create_empty_directory};
use super::worktree::{SingleWorktree, resolve_commit};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Serialize)]
struct ProfileMetadata<'a> {
    schema: &'static str,
    contract: &'static str,
    source_sha: &'a str,
    lockfile_sha256: &'a str,
    overlay_sha256: &'a str,
    overlay_paths: &'a [String],
    scenario_id: &'a str,
    sampling_profile: &'static str,
    cargo_profile_bench_debug: &'static str,
    cargo_profile_bench_strip: &'static str,
    evidence: [&'static str; 5],
}

pub(super) fn run(repository: &Path, arguments: &[String]) -> Result<(), String> {
    if !cfg!(target_os = "linux") {
        return Err(
            "`perf-profile` requires Linux perf, flamegraph, objdump, Valgrind, and GNU time"
                .into(),
        );
    }
    let arguments = ProfileArgs::parse(arguments)?;
    let artifact_dir = absolute_output_path(&arguments.artifact_dir)?;
    create_empty_directory(&artifact_dir)?;
    require_tool("perf", &["--version"])?;
    require_tool("flamegraph", &["--version"])?;
    require_tool("objdump", &["--version"])?;
    require_tool("valgrind", &["--version"])?;
    require_tool("/usr/bin/time", &["--version"])?;

    let commit = resolve_commit(repository, &arguments.commit)?;
    let checkout = SingleWorktree::create(repository, &commit)?;
    let executable =
        build_canonical_benchmark(&checkout.worktree.path, &checkout.worktree.target_dir)?;

    let perf_artifacts = artifact_dir.join("canonical-perf-stat");
    run_tool(
        "perf stat",
        Command::new("perf")
            .args(["stat", "--field-separator", ",", "--output"])
            .arg(artifact_dir.join("perf-stat.csv"))
            .arg("--")
            .arg(&executable)
            .args(benchmark_arguments(
                &arguments.scenario,
                &commit,
                &perf_artifacts,
                0,
            )),
        None,
    )?;

    let flamegraph_artifacts = artifact_dir.join("canonical-flamegraph");
    run_tool(
        "flamegraph",
        Command::new("flamegraph")
            .arg("--output")
            .arg(artifact_dir.join("flamegraph.svg"))
            .arg("--")
            .arg(&executable)
            .args(benchmark_arguments(
                &arguments.scenario,
                &commit,
                &flamegraph_artifacts,
                1,
            )),
        None,
    )?;

    let assembly = fs::File::create(artifact_dir.join("assembly.txt"))
        .map_err(|error| format!("cannot create assembly evidence: {error}"))?;
    run_tool(
        "objdump",
        Command::new("objdump")
            .args(["--demangle", "--disassemble", "--line-numbers"])
            .arg(&executable),
        Some(Stdio::from(assembly)),
    )?;

    let dhat_artifacts = artifact_dir.join("canonical-dhat");
    run_tool(
        "Valgrind DHAT",
        Command::new("valgrind")
            .arg("--tool=dhat")
            .arg(format!(
                "--dhat-out-file={}",
                artifact_dir.join("dhat.json").display()
            ))
            .arg("--")
            .arg(&executable)
            .args(benchmark_arguments(
                &arguments.scenario,
                &commit,
                &dhat_artifacts,
                2,
            )),
        None,
    )?;

    let rss_artifacts = artifact_dir.join("canonical-rss");
    run_tool(
        "GNU time RSS",
        Command::new("/usr/bin/time")
            .args(["--verbose", "--output"])
            .arg(artifact_dir.join("rss.txt"))
            .arg("--")
            .arg(&executable)
            .args(benchmark_arguments(
                &arguments.scenario,
                &commit,
                &rss_artifacts,
                3,
            )),
        None,
    )?;

    for evidence in [
        "perf-stat.csv",
        "flamegraph.svg",
        "assembly.txt",
        "dhat.json",
        "rss.txt",
    ] {
        let path = artifact_dir.join(evidence);
        let length = fs::metadata(&path)
            .map_err(|error| format!("missing profiling evidence {}: {error}", path.display()))?
            .len();
        if length == 0 {
            return Err(format!("profiling evidence is empty: {}", path.display()));
        }
    }
    checkout.worktree.assert_lock_unchanged()?;
    let metadata = ProfileMetadata {
        schema: "dol-perf-profile/v1",
        contract: "closure-v1",
        source_sha: &commit,
        lockfile_sha256: &checkout.worktree.lock_sha256,
        overlay_sha256: &checkout.worktree.overlay.sha256,
        overlay_paths: &checkout.worktree.overlay.paths,
        scenario_id: &arguments.scenario,
        sampling_profile: "adaptive",
        cargo_profile_bench_debug: "2",
        cargo_profile_bench_strip: "none",
        evidence: [
            "perf-stat.csv",
            "flamegraph.svg",
            "assembly.txt",
            "dhat.json",
            "rss.txt",
        ],
    };
    let rendered = serde_json::to_vec_pretty(&metadata)
        .map_err(|error| format!("cannot serialize profiling metadata: {error}"))?;
    fs::write(artifact_dir.join("profile-metadata.json"), rendered)
        .map_err(|error| format!("cannot write profiling metadata: {error}"))?;
    eprintln!("profiling evidence written to {}", artifact_dir.display());
    Ok(())
}

fn build_canonical_benchmark(worktree: &Path, target_dir: &Path) -> Result<PathBuf, String> {
    let output = Command::new("cargo")
        .current_dir(worktree)
        .env("CARGO_TARGET_DIR", target_dir)
        .env("CARGO_PROFILE_BENCH_DEBUG", "2")
        .env("CARGO_PROFILE_BENCH_STRIP", "none")
        .args([
            "bench",
            "--locked",
            "--package",
            "dol-bench",
            "--bench",
            "closure_v1_overlay",
            "--no-run",
            "--message-format=json-render-diagnostics",
        ])
        .output()
        .map_err(|error| format!("cannot compile profiling benchmark: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "profiling benchmark compilation failed with {}:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let stdout = String::from_utf8(output.stdout)
        .map_err(|_| "cargo returned non-UTF-8 JSON output".to_string())?;
    stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find_map(|message| {
            (message.get("reason")?.as_str()? == "compiler-artifact"
                && message.get("target")?.get("name")?.as_str()? == "closure_v1_overlay")
                .then(|| message.get("executable")?.as_str().map(PathBuf::from))
                .flatten()
        })
        .ok_or_else(|| {
            "cargo did not report the closure_v1_overlay benchmark executable".to_string()
        })
}

fn benchmark_arguments(
    scenario: &str,
    source_sha: &str,
    artifact_dir: &Path,
    order: u64,
) -> Vec<String> {
    vec![
        "--profile".into(),
        "adaptive".into(),
        "--scenario".into(),
        scenario.into(),
        "--artifact-dir".into(),
        artifact_dir.display().to_string(),
        "--side".into(),
        "single".into(),
        "--pair".into(),
        "0".into(),
        "--pair-order".into(),
        "baseline-first".into(),
        "--order".into(),
        order.to_string(),
        "--attempt".into(),
        "0".into(),
        "--source-sha".into(),
        source_sha.into(),
    ]
}

fn require_tool(program: &str, arguments: &[&str]) -> Result<(), String> {
    match Command::new(program)
        .args(arguments)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(format!(
            "required profiling tool `{program}` exited with {status}"
        )),
        Err(error) => Err(format!(
            "required profiling tool `{program}` is unavailable: {error}"
        )),
    }
}

fn run_tool(label: &str, command: &mut Command, stdout: Option<Stdio>) -> Result<(), String> {
    command
        .env("CARGO_PROFILE_BENCH_DEBUG", "2")
        .env("CARGO_PROFILE_BENCH_STRIP", "none");
    if let Some(stdout) = stdout {
        command.stdout(stdout);
    } else {
        command.stdout(Stdio::null());
    }
    let status = command
        .status()
        .map_err(|error| format!("cannot start {label}: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{label} exited with {status}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiler_benchmark_arguments_are_explicit_and_single_scenario() {
        let arguments = benchmark_arguments(
            "wire.decode.v1.84b",
            "0123456789012345678901234567890123456789",
            Path::new("evidence"),
            4,
        );
        assert!(
            arguments
                .windows(2)
                .any(|pair| pair == ["--scenario", "wire.decode.v1.84b"])
        );
        assert!(
            arguments
                .windows(2)
                .any(|pair| pair == ["--side", "single"])
        );
    }
}
