use core::cmp::Ordering;
use core::ops::Range;

use dol_core::diagnostic::{Diagnostic, Result};
use dol_core::model::Presence;
use dol_core::plan::{
    LogicalProjection, LogicalWindow, RowsFrame, SortDirection, SortExpr, WindowFrameBound,
    WindowFunctionKind,
};
use dol_core::semantics::compare_datums;
use dol_core::value::{Datum, Value, validate_datum};
use dol_engine::ExecutionRow;

use super::Session;
use super::aggregate::evaluate_sum_for_rows;
use super::row::{ScopeRecord, WorkingRow};

pub(super) fn execute_window(
    session: &mut Session<'_>,
    input: &[WorkingRow],
    outer: &[ScopeRecord],
    window: &LogicalWindow,
) -> Result<Vec<WorkingRow>> {
    let partitions = build_partitions(session, input, outer, window.partition())?;
    let mut values = vec![None; input.len()];

    for partition in partitions {
        session.check_timeout()?;
        let ordered = order_partition(session, input, outer, &partition, window.order())?;
        match window.kind() {
            WindowFunctionKind::RowNumber => assign_row_number(&ordered, &mut values)?,
            WindowFunctionKind::Rank => assign_rank(&ordered, &mut values, false)?,
            WindowFunctionKind::DenseRank => assign_rank(&ordered, &mut values, true)?,
            WindowFunctionKind::Sum => {
                assign_sum(
                    session,
                    SumWindowRequest {
                        input,
                        outer,
                        ordered: &ordered,
                        window,
                        output: &mut values,
                    },
                )?;
            }
        }
    }

    let mut output = Vec::with_capacity(input.len());
    let mut output_bytes = 0_u64;
    for (index, row) in input.iter().enumerate() {
        session.check_timeout()?;
        let datum = values
            .get_mut(index)
            .and_then(Option::take)
            .ok_or_else(|| {
                Diagnostic::error(
                    "MEMORY-WINDOW-001",
                    "window execution did not produce one value per input row",
                )
            })?;
        validate_datum(window.type_def(), Presence::Required, &datum)?;
        session.push_stage_row(
            &mut output,
            &mut output_bytes,
            WorkingRow {
                output: ExecutionRow::Product(
                    vec![row.output.clone(), ExecutionRow::Value(datum)].into_boxed_slice(),
                ),
                scopes: row.scopes.clone(),
            },
        )?;
    }
    Ok(output)
}

struct Partition {
    key: Option<Datum>,
    indices: Vec<usize>,
}

fn build_partitions(
    session: &mut Session<'_>,
    input: &[WorkingRow],
    outer: &[ScopeRecord],
    projection: Option<&LogicalProjection>,
) -> Result<Vec<Partition>> {
    let Some(projection) = projection else {
        return Ok(vec![Partition {
            key: None,
            indices: (0..input.len()).collect(),
        }]);
    };

    let mut partitions: Vec<Partition> = Vec::new();
    for (index, row) in input.iter().enumerate() {
        session.check_timeout()?;
        let key = session.evaluate_projection(projection, row, outer)?;
        if let Some(partition) = partitions
            .iter_mut()
            .find(|partition| partition.key.as_ref() == Some(&key))
        {
            partition.indices.push(index);
        } else {
            partitions.push(Partition {
                key: Some(key),
                indices: vec![index],
            });
        }
    }
    Ok(partitions)
}

struct OrderedRow {
    input_index: usize,
    original_position: usize,
    keys: Vec<Datum>,
}

fn order_partition(
    session: &mut Session<'_>,
    input: &[WorkingRow],
    outer: &[ScopeRecord],
    partition: &Partition,
    order: &[SortExpr],
) -> Result<Vec<OrderedRow>> {
    let mut rows = Vec::with_capacity(partition.indices.len());
    for (position, input_index) in partition.indices.iter().copied().enumerate() {
        session.check_timeout()?;
        let row = input.get(input_index).ok_or_else(|| {
            Diagnostic::error(
                "MEMORY-WINDOW-002",
                "window partition references an invalid row",
            )
        })?;
        let keys = order
            .iter()
            .map(|sort| session.evaluate_expr(sort.expression(), row, outer))
            .collect::<Result<Vec<_>>>()?;
        for key in &keys {
            if compare_datums(key, key).is_none() {
                return Err(Diagnostic::error(
                    "MEMORY-WINDOW-003",
                    "window order key did not provide DOL ordering semantics",
                ));
            }
        }
        rows.push(OrderedRow {
            input_index,
            original_position: position,
            keys,
        });
    }
    if !order.is_empty() {
        rows.sort_by(|left, right| compare_ordered_rows(left, right, order));
        session.check_timeout()?;
    }
    Ok(rows)
}

fn compare_ordered_rows(left: &OrderedRow, right: &OrderedRow, order: &[SortExpr]) -> Ordering {
    for ((left_key, right_key), sort) in left.keys.iter().zip(&right.keys).zip(order) {
        let ordering = compare_datums(left_key, right_key).unwrap_or(Ordering::Equal);
        let ordering = match sort.direction() {
            SortDirection::Ascending => ordering,
            SortDirection::Descending => ordering.reverse(),
        };
        if !ordering.is_eq() {
            return ordering;
        }
    }
    left.original_position.cmp(&right.original_position)
}

fn assign_row_number(ordered: &[OrderedRow], output: &mut [Option<Datum>]) -> Result<()> {
    for (position, row) in ordered.iter().enumerate() {
        store_rank(output, row.input_index, position.saturating_add(1))?;
    }
    Ok(())
}

fn assign_rank(ordered: &[OrderedRow], output: &mut [Option<Datum>], dense: bool) -> Result<()> {
    let mut rank = 1_usize;
    let mut dense_rank = 1_usize;
    for (position, row) in ordered.iter().enumerate() {
        if position > 0 {
            let previous = ordered.get(position - 1).ok_or_else(|| {
                Diagnostic::error("MEMORY-WINDOW-004", "window rank lost its previous peer")
            })?;
            if previous.keys != row.keys {
                rank = position.saturating_add(1);
                dense_rank = dense_rank.saturating_add(1);
            }
        }
        store_rank(
            output,
            row.input_index,
            if dense { dense_rank } else { rank },
        )?;
    }
    Ok(())
}

fn store_rank(output: &mut [Option<Datum>], index: usize, rank: usize) -> Result<()> {
    let rank = u64::try_from(rank).map_err(|_| {
        Diagnostic::error(
            "MEMORY-WINDOW-005",
            "window rank exceeds portable u64 range",
        )
    })?;
    let slot = output.get_mut(index).ok_or_else(|| {
        Diagnostic::error(
            "MEMORY-WINDOW-006",
            "window result index is outside the input",
        )
    })?;
    *slot = Some(Datum::Value(Value::UInt(u128::from(rank))));
    Ok(())
}

struct SumWindowRequest<'a> {
    input: &'a [WorkingRow],
    outer: &'a [ScopeRecord],
    ordered: &'a [OrderedRow],
    window: &'a LogicalWindow,
    output: &'a mut [Option<Datum>],
}

fn assign_sum(session: &mut Session<'_>, request: SumWindowRequest<'_>) -> Result<()> {
    let expression = request.window.input().ok_or_else(|| {
        Diagnostic::error(
            "MEMORY-WINDOW-007",
            "window sum is missing its input expression",
        )
    })?;
    let frame = request.window.frame().ok_or_else(|| {
        Diagnostic::error(
            "MEMORY-WINDOW-008",
            "window sum is missing its explicit rows frame",
        )
    })?;

    for position in 0..request.ordered.len() {
        session.check_timeout()?;
        let range = frame_range(frame, position, request.ordered.len());
        let mut frame_rows = Vec::with_capacity(range.len());
        for ordered_index in range {
            let input_index = request
                .ordered
                .get(ordered_index)
                .ok_or_else(|| {
                    Diagnostic::error(
                        "MEMORY-WINDOW-009",
                        "window frame references an invalid ordered row",
                    )
                })?
                .input_index;
            frame_rows.push(request.input.get(input_index).ok_or_else(|| {
                Diagnostic::error(
                    "MEMORY-WINDOW-010",
                    "window frame references an invalid input row",
                )
            })?);
        }
        let datum = evaluate_sum_for_rows(session, &frame_rows, request.outer, expression)?;
        let input_index = request
            .ordered
            .get(position)
            .ok_or_else(|| {
                Diagnostic::error("MEMORY-WINDOW-011", "window position is outside partition")
            })?
            .input_index;
        let slot = request.output.get_mut(input_index).ok_or_else(|| {
            Diagnostic::error(
                "MEMORY-WINDOW-006",
                "window result index is outside the input",
            )
        })?;
        *slot = Some(datum);
    }
    Ok(())
}

fn frame_range(frame: RowsFrame, position: usize, len: usize) -> Range<usize> {
    let start = frame_start(frame.start(), position, len);
    let end = frame_end_exclusive(frame.end(), position, len);
    start.min(end)..end
}

fn frame_start(bound: WindowFrameBound, position: usize, len: usize) -> usize {
    match bound {
        WindowFrameBound::UnboundedPreceding => 0,
        WindowFrameBound::Preceding(distance) => {
            position.saturating_sub(usize::try_from(distance).unwrap_or(usize::MAX))
        }
        WindowFrameBound::CurrentRow => position.min(len),
        WindowFrameBound::Following(distance) => position
            .saturating_add(usize::try_from(distance).unwrap_or(usize::MAX))
            .min(len),
        WindowFrameBound::UnboundedFollowing => len,
    }
}

fn frame_end_exclusive(bound: WindowFrameBound, position: usize, len: usize) -> usize {
    match bound {
        WindowFrameBound::UnboundedPreceding => 0,
        WindowFrameBound::Preceding(distance) => {
            let distance = usize::try_from(distance).unwrap_or(usize::MAX);
            if distance > position {
                0
            } else {
                position.saturating_sub(distance).saturating_add(1).min(len)
            }
        }
        WindowFrameBound::CurrentRow => position.saturating_add(1).min(len),
        WindowFrameBound::Following(distance) => position
            .saturating_add(usize::try_from(distance).unwrap_or(usize::MAX))
            .saturating_add(1)
            .min(len),
        WindowFrameBound::UnboundedFollowing => len,
    }
}
