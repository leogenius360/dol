//! Transaction-capability conformance helpers.

use dol_core::diagnostic::{Diagnostic, Result};
use dol_engine::Engine;

/// Validates the capability implications of DOL's portable transaction contract.
pub fn validate_transaction_capabilities<E>(engine: &E) -> Result<()>
where
    E: Engine,
{
    let capabilities = &engine.capabilities().transaction;
    if capabilities.transactions.is_supported() && !capabilities.atomic_writes.is_supported() {
        return Err(Diagnostic::error(
            "CONFORMANCE-TX-001",
            "supported transactions must atomically commit or roll back their writes",
        ));
    }
    Ok(())
}
