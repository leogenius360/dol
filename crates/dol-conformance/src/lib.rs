#![forbid(unsafe_code)]
//! Reusable semantic conformance harness for DOL execution engines.

pub mod differential;
pub mod migration;
pub mod pipeline;
pub mod semantic;
pub mod transaction;
pub mod write;

use dol_core::diagnostic::{Diagnostic, Result};
use dol_engine::{Engine, Support};

/// Validates engine identity and the structural invariants of its capability matrix.
pub fn validate_engine_basics<E: Engine>(engine: &E) -> Result<()> {
    if engine.info().kind.trim().is_empty() {
        return Err(Diagnostic::error(
            "CONFORMANCE-ENGINE-001",
            "engine kind must not be empty",
        ));
    }
    if engine.info().version.trim().is_empty() {
        return Err(Diagnostic::error(
            "CONFORMANCE-ENGINE-002",
            "engine version must not be empty",
        ));
    }

    let capabilities = engine.capabilities();
    let supports = [
        &capabilities.pipeline.source,
        &capabilities.pipeline.filter,
        &capabilities.pipeline.project,
        &capabilities.pipeline.aggregate,
        &capabilities.pipeline.unnest,
        &capabilities.pipeline.window,
        &capabilities.pipeline.sort,
        &capabilities.pipeline.distinct,
        &capabilities.pipeline.slice,
        &capabilities.pipeline.join,
        &capabilities.pipeline.set,
        &capabilities.pipeline.exists,
        &capabilities.writes.insert,
        &capabilities.writes.insert_many,
        &capabilities.writes.update,
        &capabilities.writes.delete,
        &capabilities.transaction.transactions,
        &capabilities.transaction.atomic_writes,
        &capabilities.transaction.referential_integrity,
    ];
    for support in supports {
        validate_support(support)?;
    }
    if capabilities.transaction.transactions.is_supported()
        && !capabilities.transaction.atomic_writes.is_supported()
    {
        return Err(Diagnostic::error(
            "CONFORMANCE-ENGINE-004",
            "a supported DOL transaction must also provide atomic writes",
        ));
    }
    Ok(())
}

fn validate_support(support: &Support) -> Result<()> {
    if let Support::Unsupported { reason } = support
        && reason.trim().is_empty()
    {
        return Err(Diagnostic::error(
            "CONFORMANCE-ENGINE-003",
            "unsupported capabilities must explain why exact semantics are unavailable",
        ));
    }
    Ok(())
}
