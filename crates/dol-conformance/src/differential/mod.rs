//! Canonical differential comparison for execution-engine result streams.
//!
//! Raw Rust equality is not sufficient for DOL conformance: floating NaNs and
//! signed zero have canonical semantic identity, and model rows must be compared
//! through their declared field types. This module compares streams using DOL's
//! typed datum fingerprints so backend transport details cannot redefine value
//! semantics.

use dol_core::diagnostic::{Diagnostic, Result};
use dol_core::fingerprint::{Fingerprint, fingerprint_datum};
use dol_core::plan::PlanOutput;
use dol_engine::{
    CollectLimits, DataStream, ExecutionRow, collect_stream, collect_stream_with_limits,
};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum RowKey {
    Model(Fingerprint, Vec<Fingerprint>),
    Value(Fingerprint),
    Product(Vec<Self>),
    Nullable(Option<Box<Self>>),
}

/// Collects and canonically compares two execution streams in result order.
///
/// The left stream is normally the memory reference oracle and the right stream
/// is the engine under conformance test. Backend batch boundaries are ignored.
pub fn compare_streams<L, R>(output: &PlanOutput, left: &mut L, right: &mut R) -> Result<()>
where
    L: DataStream + ?Sized,
    R: DataStream + ?Sized,
{
    compare_streams_with_limits(output, left, right, CollectLimits::default())
}

/// Collects and compares two ordered streams under explicit materialization limits.
pub fn compare_streams_with_limits<L, R>(
    output: &PlanOutput,
    left: &mut L,
    right: &mut R,
    limits: CollectLimits,
) -> Result<()>
where
    L: DataStream + ?Sized,
    R: DataStream + ?Sized,
{
    let left = collect_keys_with_limits(output, left, limits)?;
    let right = collect_keys_with_limits(output, right, limits)?;
    compare_keys(&left, &right)
}

/// Compares two streams as unordered multisets of canonical logical rows.
///
/// This is appropriate for plans that do not establish an ordering contract.
/// Duplicate multiplicity remains significant.
pub fn compare_streams_unordered<L, R>(
    output: &PlanOutput,
    left: &mut L,
    right: &mut R,
) -> Result<()>
where
    L: DataStream + ?Sized,
    R: DataStream + ?Sized,
{
    compare_streams_unordered_with_limits(output, left, right, CollectLimits::default())
}

/// Compares two unordered streams under explicit materialization limits.
pub fn compare_streams_unordered_with_limits<L, R>(
    output: &PlanOutput,
    left: &mut L,
    right: &mut R,
    limits: CollectLimits,
) -> Result<()>
where
    L: DataStream + ?Sized,
    R: DataStream + ?Sized,
{
    let mut left = collect_keys_with_limits(output, left, limits)?;
    let mut right = collect_keys_with_limits(output, right, limits)?;
    left.sort();
    right.sort();
    compare_keys(&left, &right)
}

/// Collects all rows while intentionally discarding backend batch boundaries.
pub fn collect_rows<S>(stream: &mut S) -> Result<Vec<ExecutionRow>>
where
    S: DataStream + ?Sized,
{
    Ok(collect_stream(stream)?.into_vec())
}

fn collect_keys_with_limits<S>(
    output: &PlanOutput,
    stream: &mut S,
    limits: CollectLimits,
) -> Result<Vec<RowKey>>
where
    S: DataStream + ?Sized,
{
    let rows = collect_stream_with_limits(stream, limits)?.into_vec();
    rows.iter().map(|row| row_key(output, row)).collect()
}

fn compare_keys(left: &[RowKey], right: &[RowKey]) -> Result<()> {
    if left.len() != right.len() {
        return Err(Diagnostic::error(
            "CONFORMANCE-DIFF-001",
            format!(
                "differential engines produced different row counts: oracle={} candidate={}",
                left.len(),
                right.len()
            ),
        ));
    }

    if let Some(index) = left
        .iter()
        .zip(right)
        .position(|(left, right)| left != right)
    {
        return Err(Diagnostic::error(
            "CONFORMANCE-DIFF-002",
            format!("differential engines disagree at canonical row index {index}"),
        ));
    }
    Ok(())
}

fn row_key(output: &PlanOutput, row: &ExecutionRow) -> Result<RowKey> {
    match (output, row) {
        (PlanOutput::Model(model), ExecutionRow::Model(row)) => {
            if row.model().fingerprint() != model.fingerprint() {
                return Err(shape_mismatch());
            }
            let fields = model
                .fields()
                .iter()
                .map(|field| {
                    let datum = row.get(field.slot()).ok_or_else(shape_mismatch)?;
                    fingerprint_datum(field.ty(), &datum.into_owned_datum())
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(RowKey::Model(model.fingerprint(), fields))
        }
        (PlanOutput::Value(ty), ExecutionRow::Value(datum)) => {
            Ok(RowKey::Value(fingerprint_datum(ty, datum)?))
        }
        (PlanOutput::Product(outputs), ExecutionRow::Product(rows))
            if outputs.len() == rows.len() =>
        {
            let values = outputs
                .iter()
                .zip(rows.iter())
                .map(|(output, row)| row_key(output, row))
                .collect::<Result<Vec<_>>>()?;
            Ok(RowKey::Product(values))
        }
        (PlanOutput::Nullable(_), ExecutionRow::Nullable(None)) => Ok(RowKey::Nullable(None)),
        (PlanOutput::Nullable(output), ExecutionRow::Nullable(Some(row))) => {
            Ok(RowKey::Nullable(Some(Box::new(row_key(output, row)?))))
        }
        _ => Err(shape_mismatch()),
    }
}

fn shape_mismatch() -> Diagnostic {
    Diagnostic::error(
        "CONFORMANCE-DIFF-003",
        "execution row shape does not match the logical plan output",
    )
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use dol_core::types::DataType;
    use dol_core::value::{Datum, Value};
    use dol_engine::ExecutionBatch;

    use super::*;

    struct TestStream {
        batches: VecDeque<ExecutionBatch>,
    }

    impl DataStream for TestStream {
        fn next_batch(&mut self) -> Result<Option<ExecutionBatch>> {
            Ok(self.batches.pop_front())
        }

        fn cancel(&mut self) {
            self.batches.clear();
        }
    }

    #[test]
    fn canonical_float_fingerprints_cover_nan_and_signed_zero() {
        let ty = f64::type_def();
        let first_nan = Datum::Value(Value::Float64(f64::from_bits(0x7ff8_0000_0000_0001)));
        let second_nan = Datum::Value(Value::Float64(f64::from_bits(0x7ff8_0000_0000_1234)));
        let zero = Datum::Value(Value::Float64(0.0));
        let negative_zero = Datum::Value(Value::Float64(-0.0));

        assert_eq!(
            fingerprint_datum(&ty, &first_nan).unwrap(),
            fingerprint_datum(&ty, &second_nan).unwrap()
        );
        assert_eq!(
            fingerprint_datum(&ty, &zero).unwrap(),
            fingerprint_datum(&ty, &negative_zero).unwrap()
        );
    }

    #[test]
    fn ordered_comparison_preserves_result_order() {
        let output = PlanOutput::Value(u64::type_def());
        let mut left = test_stream(&output, [1, 2]);
        let mut right = test_stream(&output, [2, 1]);
        let error = compare_streams(&output, &mut left, &mut right).unwrap_err();
        assert_eq!(error.code(), "CONFORMANCE-DIFF-002");
    }

    #[test]
    fn unordered_comparison_ignores_order_but_preserves_duplicate_multiplicity() {
        let output = PlanOutput::Value(u64::type_def());
        let mut left = test_stream(&output, [1, 2, 2]);
        let mut reordered = test_stream(&output, [2, 1, 2]);
        compare_streams_unordered(&output, &mut left, &mut reordered).unwrap();

        let mut left = test_stream(&output, [1, 2, 2]);
        let mut different = test_stream(&output, [1, 1, 2]);
        let error = compare_streams_unordered(&output, &mut left, &mut different).unwrap_err();
        assert_eq!(error.code(), "CONFORMANCE-DIFF-002");
    }

    #[test]
    fn differential_collection_is_bounded_and_cancels_the_offending_stream() {
        let output = PlanOutput::Value(u64::type_def());
        let mut left = test_stream(&output, [1, 2]);
        let mut right = test_stream(&output, [1, 2]);
        let error = compare_streams_with_limits(
            &output,
            &mut left,
            &mut right,
            CollectLimits {
                max_rows: 1,
                max_bytes: u64::MAX,
            },
        )
        .unwrap_err();
        assert_eq!(error.code(), "ENGINE-COLLECT-001");
        assert!(left.batches.is_empty());
    }

    fn test_stream(output: &PlanOutput, values: impl IntoIterator<Item = u128>) -> TestStream {
        let rows = values
            .into_iter()
            .map(|value| ExecutionRow::Value(Datum::Value(Value::UInt(value))))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        TestStream {
            batches: VecDeque::from([ExecutionBatch::new(output.clone(), rows)]),
        }
    }
}
