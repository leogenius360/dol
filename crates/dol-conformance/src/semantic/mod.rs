//! Result-stream and semantic execution conformance helpers.

use dol_core::data::DataSet;
use dol_core::diagnostic::{Diagnostic, Result};
use dol_core::plan::PlanOutput;
use dol_engine::{DataStream, ExecutionRow};

/// Collects a stream while checking output-shape stability and terminal behavior.
pub fn collect_checked<S>(stream: &mut S, expected: &PlanOutput) -> Result<DataSet<ExecutionRow>>
where
    S: DataStream + ?Sized,
{
    let mut rows = Vec::new();
    while let Some(batch) = stream.next_batch()? {
        if batch.output() != expected {
            return Err(Diagnostic::error(
                "CONFORMANCE-STREAM-001",
                "execution batch changed logical output shape within one stream",
            ));
        }
        rows.extend(batch.into_rows());
    }
    if stream.next_batch()?.is_some() {
        return Err(Diagnostic::error(
            "CONFORMANCE-STREAM-002",
            "completed stream produced rows after terminal None",
        ));
    }
    Ok(DataSet::new(rows))
}
