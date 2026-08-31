//! Pipeline capability and placement conformance helpers.

use dol_core::diagnostic::{Diagnostic, Result};
use dol_core::plan::LogicalPlan;
use dol_engine::{Engine, PlacementPlan, PlacementPolicy, analyze_engine_placement};

/// Requires every node in a plan to receive exact remote-only placement.
pub fn validate_remote_only_placement<E>(engine: &E, plan: &LogicalPlan) -> Result<PlacementPlan>
where
    E: Engine,
{
    let placement = analyze_engine_placement(plan, engine, PlacementPolicy::RemoteOnly)?;
    if !placement.is_fully_engine() {
        return Err(Diagnostic::error(
            "CONFORMANCE-PIPELINE-001",
            "remote-only placement returned a local residual node",
        ));
    }
    Ok(placement)
}
