//! Write-capability conformance helpers.

use dol_core::diagnostic::{Diagnostic, Result};
use dol_core::ops::WriteKind;
use dol_engine::{Engine, Support};

/// Requires exact support for one first-class write family.
pub fn require_exact_write<E>(engine: &E, kind: WriteKind) -> Result<()>
where
    E: Engine,
{
    let capabilities = &engine.capabilities().writes;
    let support = match kind {
        WriteKind::Insert => &capabilities.insert,
        WriteKind::InsertMany => &capabilities.insert_many,
        WriteKind::Update => &capabilities.update,
        WriteKind::Delete => &capabilities.delete,
    };
    if matches!(support, Support::ExactNative | Support::ExactEmulated) {
        Ok(())
    } else {
        Err(Diagnostic::error(
            "CONFORMANCE-WRITE-001",
            format!(
                "engine does not advertise exact {:?} semantics: {}",
                kind,
                support.reason().unwrap_or("unsupported")
            ),
        ))
    }
}
