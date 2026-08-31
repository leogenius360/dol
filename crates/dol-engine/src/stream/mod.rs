//! Pull-based execution stream and backend-independent materialized row shape.

use dol_core::data::DataSet;
use dol_core::diagnostic::Result;
use dol_core::plan::PlanOutput;
use dol_core::runtime::DynRow;
use dol_core::value::Datum;

/// One dynamically materialized logical-plan row.
///
/// This mirrors [`PlanOutput`] after Rust generic result information has been erased
/// at the engine boundary. It is execution support data, not a new DOL computation
/// abstraction; collected values still live in [`DataSet`].
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum ExecutionRow {
    /// Complete semantic model row.
    Model(DynRow),
    /// Scalar, tuple, record, list, or other logical value.
    Value(Datum),
    /// Ordered product produced by joins/windows.
    Product(Box<[ExecutionRow]>),
    /// Nullable output side, absent for an unmatched outer-join row.
    Nullable(Option<Box<ExecutionRow>>),
}

impl ExecutionRow {
    /// Approximate owned logical payload bytes used by execution limits.
    #[doc(hidden)]
    #[must_use]
    pub fn logical_bytes(&self) -> u64 {
        match self {
            Self::Model(row) => row.logical_bytes(),
            Self::Value(datum) => datum.logical_bytes(),
            Self::Product(values) => values.iter().fold(8_u64, |size, value| {
                size.saturating_add(value.logical_bytes())
            }),
            Self::Nullable(None) => 1,
            Self::Nullable(Some(value)) => 1_u64.saturating_add(value.logical_bytes()),
        }
    }
}

/// One pull-based execution batch.
#[derive(Debug, Clone)]
pub struct ExecutionBatch {
    output: PlanOutput,
    rows: Box<[ExecutionRow]>,
}

impl ExecutionBatch {
    /// Creates a validated batch container.
    #[must_use]
    pub fn new(output: PlanOutput, rows: Box<[ExecutionRow]>) -> Self {
        Self { output, rows }
    }

    /// Logical output shape shared by every row.
    #[must_use]
    pub const fn output(&self) -> &PlanOutput {
        &self.output
    }

    /// Materialized rows in engine result order.
    #[must_use]
    pub fn rows(&self) -> &[ExecutionRow] {
        &self.rows
    }

    /// Consumes the batch into its rows.
    #[must_use]
    pub fn into_rows(self) -> Box<[ExecutionRow]> {
        self.rows
    }
}

/// Synchronous pull stream with explicit cancellation.
///
/// Engines may do work lazily between `next_batch` calls. The pull contract gives
/// the caller natural backpressure without forcing one async runtime into DOL's SPI.
pub trait DataStream {
    /// Returns the next batch or `None` after completion.
    fn next_batch(&mut self) -> Result<Option<ExecutionBatch>>;

    /// Requests cancellation. Further pulls must not produce new rows.
    fn cancel(&mut self);
}

/// Hard limits for eager stream collection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CollectLimits {
    /// Maximum collected rows.
    pub max_rows: u64,
    /// Maximum approximate logical payload bytes.
    pub max_bytes: u64,
}

impl Default for CollectLimits {
    fn default() -> Self {
        Self {
            max_rows: 100_000,
            max_bytes: 64 * 1024 * 1024,
        }
    }
}

/// Collects a pull stream under conservative default materialization limits.
pub fn collect_stream<S>(stream: &mut S) -> Result<DataSet<ExecutionRow>>
where
    S: DataStream + ?Sized,
{
    collect_stream_with_limits(stream, CollectLimits::default())
}

/// Collects a pull stream under explicit row and logical-byte limits.
pub fn collect_stream_with_limits<S>(
    stream: &mut S,
    limits: CollectLimits,
) -> Result<DataSet<ExecutionRow>>
where
    S: DataStream + ?Sized,
{
    let mut rows = Vec::new();
    let mut row_count = 0_u64;
    let mut logical_bytes = 0_u64;
    while let Some(batch) = stream.next_batch()? {
        let batch = batch.into_rows();
        let batch_rows = u64::try_from(batch.len()).map_err(|_| {
            dol_core::diagnostic::Diagnostic::error(
                "ENGINE-COLLECT-001",
                "stream batch row count exceeds the portable u64 domain",
            )
        })?;
        row_count = row_count.checked_add(batch_rows).ok_or_else(|| {
            dol_core::diagnostic::Diagnostic::error(
                "ENGINE-COLLECT-001",
                "collected stream row count overflow",
            )
        })?;
        if row_count > limits.max_rows {
            stream.cancel();
            return Err(dol_core::diagnostic::Diagnostic::error(
                "ENGINE-COLLECT-001",
                "stream exceeds the configured eager collection row limit",
            ));
        }
        for row in &batch {
            logical_bytes = logical_bytes.saturating_add(row.logical_bytes());
            if logical_bytes > limits.max_bytes {
                stream.cancel();
                return Err(dol_core::diagnostic::Diagnostic::error(
                    "ENGINE-COLLECT-002",
                    "stream exceeds the configured eager collection byte limit",
                ));
            }
        }
        rows.extend(batch);
    }
    Ok(DataSet::new(rows))
}

#[cfg(test)]
mod tests {
    use dol_core::diagnostic::Result;
    use dol_core::plan::PlanOutput;
    use dol_core::types::{ScalarRepr, TypeDef};
    use dol_core::value::{Datum, Value};

    use super::{
        CollectLimits, DataStream, ExecutionBatch, ExecutionRow, collect_stream_with_limits,
    };

    struct Stream {
        batch: Option<ExecutionBatch>,
        cancelled: bool,
    }

    impl DataStream for Stream {
        fn next_batch(&mut self) -> Result<Option<ExecutionBatch>> {
            Ok(self.batch.take())
        }

        fn cancel(&mut self) {
            self.cancelled = true;
            self.batch = None;
        }
    }

    #[test]
    fn eager_collection_is_bounded_and_cancels_upstream() {
        let ty = TypeDef::scalar("test/u64", 1, ScalarRepr::UInt { bits: 64 });
        let rows = vec![
            ExecutionRow::Value(Datum::Value(Value::UInt(1))),
            ExecutionRow::Value(Datum::Value(Value::UInt(2))),
        ];
        let mut stream = Stream {
            batch: Some(ExecutionBatch::new(
                PlanOutput::Value(ty),
                rows.into_boxed_slice(),
            )),
            cancelled: false,
        };
        let error = collect_stream_with_limits(
            &mut stream,
            CollectLimits {
                max_rows: 1,
                max_bytes: u64::MAX,
            },
        )
        .unwrap_err();
        assert_eq!(error.code(), "ENGINE-COLLECT-001");
        assert!(stream.cancelled);
    }
}
