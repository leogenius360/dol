use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::fs;
use std::io::Read as _;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use tempfile::TempDir;

#[derive(Clone, Debug)]
pub(super) struct OverlayEvidence {
    pub(super) sha256: String,
    pub(super) paths: Vec<String>,
}

pub(super) struct DetachedWorktree {
    repository: PathBuf,
    pub(super) path: PathBuf,
    pub(super) target_dir: PathBuf,
    original_lock: Vec<u8>,
    pub(super) lock_sha256: String,
    pub(super) overlay: OverlayEvidence,
}

impl DetachedWorktree {
    fn create(repository: &Path, scratch: &Path, name: &str, commit: &str) -> Result<Self, String> {
        let path = scratch.join(name);
        let target_dir = scratch.join(format!("target-{name}"));
        let status = Command::new("git")
            .current_dir(repository)
            .args(["worktree", "add", "--detach"])
            .arg(&path)
            .arg(commit)
            .stdout(Stdio::null())
            .status()
            .map_err(|error| format!("cannot create detached {name} worktree: {error}"))?;
        if !status.success() {
            return Err(format!(
                "cannot create detached {name} worktree at {}: git exited with {status}",
                path.display()
            ));
        }
        let original_lock = fs::read(path.join("Cargo.lock"))
            .map_err(|error| format!("cannot read {name} Cargo.lock: {error}"))?;
        let lock_sha256 = sha256_bytes(&original_lock);
        let overlay = match install_overlay(repository, &path) {
            Ok(overlay) => overlay,
            Err(error) => {
                remove_worktree(repository, &path);
                return Err(error);
            }
        };
        Ok(Self {
            repository: repository.to_owned(),
            path,
            target_dir,
            original_lock,
            lock_sha256,
            overlay,
        })
    }

    pub(super) fn assert_lock_unchanged(&self) -> Result<(), String> {
        let current = fs::read(self.path.join("Cargo.lock"))
            .map_err(|error| format!("cannot re-read Cargo.lock: {error}"))?;
        if current == self.original_lock {
            Ok(())
        } else {
            Err(format!(
                "comparison invalid: {} Cargo.lock changed (original {}, current {})",
                self.path.display(),
                self.lock_sha256,
                sha256_bytes(&current)
            ))
        }
    }
}

impl Drop for DetachedWorktree {
    fn drop(&mut self) {
        remove_worktree(&self.repository, &self.path);
    }
}

pub(super) struct WorktreePair {
    pub(super) baseline: DetachedWorktree,
    pub(super) candidate: DetachedWorktree,
    _scratch: TempDir,
}

impl WorktreePair {
    pub(super) fn create(
        repository: &Path,
        baseline: &str,
        candidate: &str,
    ) -> Result<Self, String> {
        let scratch = tempfile::Builder::new()
            .prefix("dol-perf-worktrees-")
            .tempdir()
            .map_err(|error| format!("cannot create performance scratch directory: {error}"))?;
        let baseline = DetachedWorktree::create(repository, scratch.path(), "baseline", baseline)?;
        let candidate =
            DetachedWorktree::create(repository, scratch.path(), "candidate", candidate)?;
        Ok(Self {
            baseline,
            candidate,
            _scratch: scratch,
        })
    }
}

pub(super) struct SingleWorktree {
    pub(super) worktree: DetachedWorktree,
    _scratch: TempDir,
}

impl SingleWorktree {
    pub(super) fn create(repository: &Path, commit: &str) -> Result<Self, String> {
        let scratch = tempfile::Builder::new()
            .prefix("dol-perf-profile-")
            .tempdir()
            .map_err(|error| format!("cannot create profiling scratch directory: {error}"))?;
        let worktree = DetachedWorktree::create(repository, scratch.path(), "profile", commit)?;
        Ok(Self {
            worktree,
            _scratch: scratch,
        })
    }
}

pub(super) fn resolve_commit(repository: &Path, revision: &str) -> Result<String, String> {
    if revision.is_empty()
        || revision.starts_with('-')
        || revision.contains(char::is_whitespace)
        || revision.contains("..")
        || revision.bytes().any(|byte| {
            !(byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'/' | b'_' | b'-'))
        })
    {
        return Err(format!("unsafe or invalid Git revision `{revision}`"));
    }
    let expression = format!("{revision}^{{commit}}");
    let output = Command::new("git")
        .current_dir(repository)
        .args(["rev-parse", "--verify", "--end-of-options"])
        .arg(expression)
        .output()
        .map_err(|error| format!("cannot resolve Git revision `{revision}`: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "Git revision `{revision}` does not resolve to a commit"
        ));
    }
    let sha = String::from_utf8(output.stdout)
        .map_err(|_| "git rev-parse returned non-UTF-8 output".to_string())?
        .trim()
        .to_ascii_lowercase();
    if sha.len() != 40 || !sha.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!(
            "git returned non-full commit ID `{sha}` for `{revision}`"
        ));
    }
    Ok(sha)
}

pub(super) fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file =
        fs::File::open(path).map_err(|error| format!("cannot hash {}: {error}", path.display()))?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("cannot hash {}: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(lower_hex(&digest.finalize()))
}

pub(super) fn sha256_bytes(bytes: &[u8]) -> String {
    lower_hex(&Sha256::digest(bytes))
}

fn lower_hex(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
}

fn install_overlay(source: &Path, destination: &Path) -> Result<OverlayEvidence, String> {
    append_normalized_bench_profile(&destination.join("Cargo.toml"))?;
    merge_overlay_manifest(
        &source.join("tools/benchmarks/Cargo.toml"),
        &destination.join("tools/benchmarks/Cargo.toml"),
    )?;

    copy_file(
        &source.join("tools/benchmarks/src/workloads.rs"),
        &destination.join("tools/benchmarks/src/workloads.rs"),
    )?;
    copy_file(
        &source.join("tools/benchmarks/benches/closure_v1_overlay.rs"),
        &destination.join("tools/benchmarks/benches/closure_v1_overlay.rs"),
    )?;
    replace_directory(
        &source.join("perf/fixtures/closure-v1"),
        &destination.join("perf/fixtures/closure-v1"),
    )?;

    let paths = changed_paths(destination)?;
    validate_overlay_paths(&paths)?;
    let sha256 = hash_overlay(destination, &paths)?;
    Ok(OverlayEvidence { sha256, paths })
}

fn append_normalized_bench_profile(manifest: &Path) -> Result<(), String> {
    let mut source = fs::read_to_string(manifest)
        .map_err(|error| format!("cannot read {}: {error}", manifest.display()))?;
    let parsed = source
        .parse::<toml::Table>()
        .map_err(|error| format!("cannot parse {}: {error}", manifest.display()))?;
    if let Some(profile) = parsed
        .get("profile")
        .and_then(toml::Value::as_table)
        .and_then(|profiles| profiles.get("bench"))
        .and_then(toml::Value::as_table)
    {
        let valid = profile
            .get("codegen-units")
            .and_then(toml::Value::as_integer)
            == Some(1)
            && profile.get("lto").and_then(toml::Value::as_str) == Some("thin")
            && profile
                .get("overflow-checks")
                .and_then(toml::Value::as_bool)
                == Some(true)
            && profile.get("strip").and_then(toml::Value::as_str) == Some("debuginfo");
        if !valid {
            return Err("revision has a conflicting [profile.bench]; comparison refused".into());
        }
        return Ok(());
    }
    if !source.ends_with('\n') {
        source.push('\n');
    }
    source.push_str(
        "\n[profile.bench]\ncodegen-units = 1\nlto = \"thin\"\noverflow-checks = true\nstrip = \"debuginfo\"\n",
    );
    fs::write(manifest, source)
        .map_err(|error| format!("cannot normalize {}: {error}", manifest.display()))
}

fn merge_overlay_manifest(current: &Path, target: &Path) -> Result<(), String> {
    let current_source = fs::read_to_string(current)
        .map_err(|error| format!("cannot read {}: {error}", current.display()))?;
    let target_source = fs::read_to_string(target)
        .map_err(|error| format!("cannot read {}: {error}", target.display()))?;
    let current = current_source
        .parse::<toml::Table>()
        .map_err(|error| format!("cannot parse {}: {error}", current.display()))?;
    let mut target_table = target_source
        .parse::<toml::Table>()
        .map_err(|error| format!("cannot parse {}: {error}", target.display()))?;

    let closure = current
        .get("bench")
        .and_then(toml::Value::as_array)
        .and_then(|benches| {
            benches.iter().find(|bench| {
                bench
                    .as_table()
                    .and_then(|table| table.get("name"))
                    .and_then(toml::Value::as_str)
                    == Some("closure_v1_overlay")
            })
        })
        .cloned()
        .ok_or_else(|| "current dol-bench manifest has no closure_v1_overlay target".to_string())?;
    let benches = target_table
        .entry("bench")
        .or_insert_with(|| toml::Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or_else(|| "historical dol-bench `bench` declaration is invalid".to_string())?;
    benches.retain(|bench| {
        bench
            .as_table()
            .and_then(|table| table.get("name"))
            .and_then(toml::Value::as_str)
            != Some("closure_v1_overlay")
    });
    benches.push(closure);

    let rendered = toml::to_string_pretty(&target_table)
        .map_err(|error| format!("cannot render canonical benchmark manifest: {error}"))?;
    fs::write(target, rendered)
        .map_err(|error| format!("cannot write {}: {error}", target.display()))
}

fn replace_directory(source: &Path, destination: &Path) -> Result<(), String> {
    if !source.is_dir() {
        return Err(format!(
            "overlay source directory is missing: {}",
            source.display()
        ));
    }
    if destination.exists() {
        fs::remove_dir_all(destination)
            .map_err(|error| format!("cannot reset {}: {error}", destination.display()))?;
    }
    copy_directory(source, destination)
}

fn copy_directory(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination)
        .map_err(|error| format!("cannot create {}: {error}", destination.display()))?;
    for entry in fs::read_dir(source)
        .map_err(|error| format!("cannot read {}: {error}", source.display()))?
    {
        let entry = entry.map_err(|error| format!("cannot read overlay entry: {error}"))?;
        let kind = entry
            .file_type()
            .map_err(|error| format!("cannot inspect {}: {error}", entry.path().display()))?;
        if kind.is_symlink() {
            return Err(format!(
                "overlay may not contain symlinks: {}",
                entry.path().display()
            ));
        }
        let target = destination.join(entry.file_name());
        if kind.is_dir() {
            copy_directory(&entry.path(), &target)?;
        } else if kind.is_file() {
            copy_file(&entry.path(), &target)?;
        } else {
            return Err(format!(
                "unsupported overlay entry: {}",
                entry.path().display()
            ));
        }
    }
    Ok(())
}

fn copy_file(source: &Path, destination: &Path) -> Result<(), String> {
    if !source.is_file() {
        return Err(format!(
            "overlay source file is missing: {}",
            source.display()
        ));
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    }
    fs::copy(source, destination).map(|_| ()).map_err(|error| {
        format!(
            "cannot overlay {} to {}: {error}",
            source.display(),
            destination.display()
        )
    })
}

fn changed_paths(worktree: &Path) -> Result<Vec<String>, String> {
    let output = Command::new("git")
        .current_dir(worktree)
        .args(["status", "--porcelain=v1", "-z", "--untracked-files=all"])
        .output()
        .map_err(|error| format!("cannot inspect overlay diff: {error}"))?;
    if !output.status.success() {
        return Err(format!("git status failed with {}", output.status));
    }
    let mut paths = Vec::new();
    for record in output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|row| !row.is_empty())
    {
        if record.len() < 4 || record[2] != b' ' {
            return Err("git status returned an invalid porcelain record".into());
        }
        let path = String::from_utf8(record[3..].to_vec())
            .map_err(|_| "overlay path is not valid UTF-8".to_string())?
            .replace('\\', "/");
        paths.push(path);
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn validate_overlay_paths(paths: &[String]) -> Result<(), String> {
    if paths.is_empty() {
        return Err("measurement overlay produced no changes".into());
    }
    for path in paths {
        validate_relative_path(path)?;
        let allowed = path == "Cargo.toml"
            || path == "tools/benchmarks/Cargo.toml"
            || path.starts_with("tools/benchmarks/src/")
            || path == "tools/benchmarks/benches/closure_v1_overlay.rs"
            || path.starts_with("perf/fixtures/closure-v1/");
        if path.starts_with("crates/") || path.starts_with("engines/") {
            return Err(format!(
                "overlay attempted to modify runtime source `{path}`"
            ));
        }
        if !allowed {
            return Err(format!(
                "overlay path `{path}` is outside the closure-v1 allowlist"
            ));
        }
    }
    Ok(())
}

fn validate_relative_path(path: &str) -> Result<(), String> {
    let path = Path::new(path);
    if path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(format!("unsafe overlay path `{}`", path.display()));
    }
    Ok(())
}

fn hash_overlay(worktree: &Path, paths: &[String]) -> Result<String, String> {
    let mut digest = Sha256::new();
    for path in paths {
        digest.update((path.len() as u64).to_le_bytes());
        digest.update(path.as_bytes());
        let content = fs::read(worktree.join(path))
            .map_err(|error| format!("cannot hash overlay path {path}: {error}"))?;
        digest.update((content.len() as u64).to_le_bytes());
        digest.update(content);
    }
    Ok(lower_hex(&digest.finalize()))
}

fn remove_worktree(repository: &Path, worktree: &Path) {
    let _ = Command::new("git")
        .current_dir(repository)
        .args(["worktree", "remove", "--force"])
        .arg(worktree)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revisions_cannot_be_interpreted_as_git_options() {
        assert!(resolve_commit(Path::new("."), "--help").is_err());
        assert!(resolve_commit(Path::new("."), "HEAD^{tree}").is_err());
    }

    #[test]
    fn overlay_allowlist_rejects_runtime_and_traversal_paths() {
        assert!(validate_overlay_paths(&["crates/dol-core/src/lib.rs".into()]).is_err());
        assert!(validate_overlay_paths(&["../Cargo.toml".into()]).is_err());
        assert!(validate_overlay_paths(&["tools/benchmarks/src/lib.rs".into()]).is_ok());
    }

    #[test]
    fn overlay_digest_commits_to_boundaries() {
        assert_ne!(sha256_bytes(&[1, 23]), sha256_bytes(&[12, 3]));
    }
}
