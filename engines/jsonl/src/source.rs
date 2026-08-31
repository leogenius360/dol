use std::collections::BTreeMap;
use std::fs::{self, File, Metadata};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use dol_core::diagnostic::{Diagnostic, Result};
use dol_core::model::{Model, ModelDef, ModelKey};

#[derive(Debug, Clone)]
pub(crate) struct JsonlSource {
    pub(crate) model: Arc<ModelDef>,
    pub(crate) path: PathBuf,
    root: PathBuf,
    file: Arc<File>,
}

impl JsonlSource {
    pub(crate) fn open_file(&self) -> Result<File> {
        let canonical = fs::canonicalize(&self.path).map_err(|error| {
            Diagnostic::error(
                "JSONL-SOURCE-005",
                format!("cannot re-resolve JSONL source before execution: {error}"),
            )
        })?;
        if !canonical.starts_with(&self.root) {
            return Err(Diagnostic::error(
                "JSONL-SOURCE-002",
                "JSONL source resolves outside the configured root at execution time",
            ));
        }
        let metadata = fs::metadata(&canonical).map_err(|error| {
            Diagnostic::error(
                "JSONL-SOURCE-005",
                format!("cannot inspect JSONL source before execution: {error}"),
            )
        })?;
        if !metadata.is_file() {
            return Err(Diagnostic::error(
                "JSONL-SOURCE-003",
                "JSONL source must resolve to a regular file",
            ));
        }
        let file = File::open(&canonical).map_err(|error| {
            Diagnostic::error(
                "JSONL-SOURCE-005",
                format!("cannot reopen pinned JSONL source: {error}"),
            )
        })?;
        let opened = file.metadata().map_err(|error| {
            Diagnostic::error(
                "JSONL-SOURCE-005",
                format!("cannot inspect reopened JSONL source: {error}"),
            )
        })?;
        let pinned = self.file.metadata().map_err(|error| {
            Diagnostic::error(
                "JSONL-SOURCE-005",
                format!("cannot inspect pinned JSONL source: {error}"),
            )
        })?;
        if !same_file(&pinned, &metadata) || !same_file(&pinned, &opened) {
            return Err(Diagnostic::error(
                "JSONL-SOURCE-005",
                "JSONL source path no longer identifies the file pinned at binding time",
            ));
        }
        Ok(file)
    }
}

pub(crate) type Sources = BTreeMap<ModelKey, JsonlSource>;

pub(crate) fn canonical_root(root: impl AsRef<Path>) -> Result<PathBuf> {
    let root = fs::canonicalize(root.as_ref()).map_err(|error| {
        Diagnostic::error(
            "JSONL-SOURCE-001",
            format!("cannot resolve JSONL root directory: {error}"),
        )
    })?;
    let metadata = fs::metadata(&root).map_err(|error| {
        Diagnostic::error(
            "JSONL-SOURCE-001",
            format!("cannot inspect JSONL root directory: {error}"),
        )
    })?;
    if !metadata.is_dir() {
        return Err(Diagnostic::error(
            "JSONL-SOURCE-001",
            "JSONL root must be a directory",
        ));
    }
    Ok(root)
}

pub(crate) fn bind_model<M>(
    sources: &mut Sources,
    root: &Path,
    relative_path: impl AsRef<Path>,
) -> Result<()>
where
    M: Model,
{
    bind_definition(sources, root, M::model_def()?.clone(), relative_path)
}

pub(crate) fn bind_definition(
    sources: &mut Sources,
    root: &Path,
    model: ModelDef,
    relative_path: impl AsRef<Path>,
) -> Result<()> {
    let path = resolve_source_path(root, relative_path.as_ref())?;
    let key = model.key().clone();
    if sources.contains_key(&key) {
        return Err(Diagnostic::error(
            "JSONL-SOURCE-004",
            format!("model `{}` already has a JSONL source", key.as_str()),
        ));
    }
    let file = File::open(&path).map_err(|error| {
        Diagnostic::error(
            "JSONL-SOURCE-003",
            format!("cannot pin JSONL source handle: {error}"),
        )
    })?;
    let opened = file.metadata().map_err(|error| {
        Diagnostic::error(
            "JSONL-SOURCE-003",
            format!("cannot inspect pinned JSONL source handle: {error}"),
        )
    })?;
    let current = fs::metadata(&path).map_err(|error| {
        Diagnostic::error(
            "JSONL-SOURCE-003",
            format!("cannot re-inspect JSONL source after opening: {error}"),
        )
    })?;
    if !same_file(&opened, &current) {
        return Err(Diagnostic::error(
            "JSONL-SOURCE-005",
            "JSONL source changed while its binding handle was opened",
        ));
    }
    sources.insert(
        key,
        JsonlSource {
            model: Arc::new(model),
            path,
            root: root.to_path_buf(),
            file: Arc::new(file),
        },
    );
    Ok(())
}

#[cfg(unix)]
fn same_file(left: &Metadata, right: &Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;

    left.dev() == right.dev() && left.ino() == right.ino()
}

#[cfg(windows)]
fn same_file(left: &Metadata, right: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    left.file_attributes() == right.file_attributes()
        && left.creation_time() == right.creation_time()
        && left.last_write_time() == right.last_write_time()
        && left.file_size() == right.file_size()
}

#[cfg(not(any(unix, windows)))]
fn same_file(left: &Metadata, right: &Metadata) -> bool {
    left.len() == right.len()
        && left.modified().ok() == right.modified().ok()
        && left.created().ok() == right.created().ok()
}

fn resolve_source_path(root: &Path, relative: &Path) -> Result<PathBuf> {
    if relative.as_os_str().is_empty() || relative.is_absolute() {
        return Err(Diagnostic::error(
            "JSONL-SOURCE-002",
            "JSONL source paths must be non-empty paths relative to the configured root",
        ));
    }

    let joined = root.join(relative);
    let canonical = fs::canonicalize(&joined).map_err(|error| {
        Diagnostic::error(
            "JSONL-SOURCE-003",
            format!(
                "cannot resolve JSONL source `{}`: {error}",
                relative.display()
            ),
        )
    })?;
    if !canonical.starts_with(root) {
        return Err(Diagnostic::error(
            "JSONL-SOURCE-002",
            "JSONL source resolves outside the configured root",
        ));
    }
    let metadata = fs::metadata(&canonical).map_err(|error| {
        Diagnostic::error(
            "JSONL-SOURCE-003",
            format!(
                "cannot inspect JSONL source `{}`: {error}",
                relative.display()
            ),
        )
    })?;
    if !metadata.is_file() {
        return Err(Diagnostic::error(
            "JSONL-SOURCE-003",
            "JSONL source must resolve to a regular file",
        ));
    }
    Ok(canonical)
}
