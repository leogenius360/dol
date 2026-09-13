mod cli;
mod compare;
mod contract;
mod execute;
mod profile;
mod runner;
mod worktree;

use std::path::PathBuf;
use std::process::{Command, Stdio};

pub(crate) fn compare(arguments: &[String]) -> Result<(), String> {
    compare::run(&repository_root()?, arguments)
}

pub(crate) fn aa_smoke(arguments: &[String]) -> Result<(), String> {
    compare::aa_smoke(&repository_root()?, arguments)
}

pub(crate) fn profile(arguments: &[String]) -> Result<(), String> {
    profile::run(&repository_root()?, arguments)
}

pub(crate) fn overlay_check() -> Result<(), String> {
    let repository = repository_root()?;
    contract::validate(
        &repository,
        contract::BASELINE_SHA,
        contract::CANDIDATE_SHA,
        cli::ComparisonProfile::Candidate,
    )?;
    let checkouts = worktree::WorktreePair::create(
        &repository,
        contract::BASELINE_SHA,
        contract::CANDIDATE_SHA,
    )?;
    for (name, checkout) in [
        ("baseline", &checkouts.baseline),
        ("candidate", &checkouts.candidate),
    ] {
        let status = Command::new("cargo")
            .current_dir(&checkout.path)
            .env("CARGO_TARGET_DIR", &checkout.target_dir)
            .args([
                "bench",
                "--locked",
                "--package",
                "dol-bench",
                "--bench",
                "closure_v1_overlay",
                "--no-run",
            ])
            .stdout(Stdio::null())
            .status()
            .map_err(|error| format!("cannot compile {name} measurement overlay: {error}"))?;
        if !status.success() {
            return Err(format!(
                "{name} measurement overlay failed to compile with its original Cargo.lock: {status}"
            ));
        }
        checkout.assert_lock_unchanged()?;
    }
    Ok(())
}

fn repository_root() -> Result<PathBuf, String> {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .map(std::path::Path::to_owned)
        .ok_or_else(|| "cannot resolve repository root from xtask manifest".to_string())
}
