use dol_core::analytics::{AggregateKind, AggregateSelectionKind};
use dol_core::diagnostic::{Diagnostic, Result};
use dol_core::model::Presence;
use dol_core::plan::{LogicalAggregate, LogicalAggregateSelection, LogicalProjection};
use dol_core::semantics::{compare_datums, sum_exact_values};
use dol_core::value::{Datum, Value, validate_datum};
use dol_engine::ExecutionRow;

use super::Session;
use super::row::{ScopeRecord, WorkingRow};

pub(super) fn execute_aggregate(
    session: &mut Session<'_>,
    input: &[WorkingRow],
    outer: &[ScopeRecord],
    groups: Option<&LogicalProjection>,
    selection: &LogicalAggregateSelection,
) -> Result<Vec<WorkingRow>> {
    let grouped = build_groups(session, input, outer, groups)?;
    let mut output = Vec::with_capacity(grouped.len());
    let mut output_bytes = 0_u64;
    for group in grouped {
        session.check_timeout()?;
        let aggregate = evaluate_selection(session, &group.rows, outer, selection)?;
        let datum = match group.key {
            Some(key) => Datum::Value(Value::Tuple(vec![key, aggregate])),
            None => aggregate,
        };
        session.push_stage_row(
            &mut output,
            &mut output_bytes,
            WorkingRow {
                output: ExecutionRow::Value(datum),
                scopes: Vec::new(),
            },
        )?;
    }
    Ok(output)
}

struct Group<'a> {
    key: Option<Datum>,
    rows: Vec<&'a WorkingRow>,
}

fn build_groups<'a>(
    session: &mut Session<'_>,
    input: &'a [WorkingRow],
    outer: &[ScopeRecord],
    groups: Option<&LogicalProjection>,
) -> Result<Vec<Group<'a>>> {
    let Some(groups) = groups else {
        return Ok(vec![Group {
            key: None,
            rows: input.iter().collect(),
        }]);
    };

    let mut result: Vec<Group<'a>> = Vec::new();
    for row in input {
        session.check_timeout()?;
        let key = session.evaluate_projection(groups, row, outer)?;
        if let Some(group) = result
            .iter_mut()
            .find(|group| group.key.as_ref() == Some(&key))
        {
            group.rows.push(row);
        } else {
            result.push(Group {
                key: Some(key),
                rows: vec![row],
            });
        }
    }
    Ok(result)
}

fn evaluate_selection(
    session: &mut Session<'_>,
    rows: &[&WorkingRow],
    outer: &[ScopeRecord],
    selection: &LogicalAggregateSelection,
) -> Result<Datum> {
    let values = selection
        .aggregates()
        .iter()
        .map(|aggregate| evaluate_one(session, rows, outer, aggregate))
        .collect::<Result<Vec<_>>>()?;

    let datum = match selection.kind() {
        AggregateSelectionKind::Value => values.into_iter().next().ok_or_else(|| {
            Diagnostic::error("MEMORY-AGG-001", "aggregate selection has no result")
        })?,
        AggregateSelectionKind::Tuple => Datum::Value(Value::Tuple(values)),
    };
    validate_datum(selection.type_def(), Presence::Required, &datum)?;
    Ok(datum)
}

fn evaluate_one(
    session: &mut Session<'_>,
    rows: &[&WorkingRow],
    outer: &[ScopeRecord],
    aggregate: &LogicalAggregate,
) -> Result<Datum> {
    let datum = match aggregate.kind() {
        AggregateKind::CountRows => Datum::Value(Value::UInt(rows.len() as u128)),
        AggregateKind::CountPresent => {
            let input = required_input(aggregate)?;
            let mut count = 0_u128;
            for row in rows {
                session.check_timeout()?;
                if matches!(session.evaluate_expr(input, row, outer)?, Datum::Value(_)) {
                    count = count.checked_add(1).ok_or_else(|| {
                        Diagnostic::error("MEMORY-AGG-002", "aggregate row count overflowed")
                    })?;
                }
            }
            Datum::Value(Value::UInt(count))
        }
        AggregateKind::Sum => {
            evaluate_sum_for_rows(session, rows, outer, required_input(aggregate)?)?
        }
        AggregateKind::Min => {
            evaluate_extreme(session, rows, outer, required_input(aggregate)?, false)?
        }
        AggregateKind::Max => {
            evaluate_extreme(session, rows, outer, required_input(aggregate)?, true)?
        }
    };
    validate_datum(aggregate.type_def(), Presence::Required, &datum)?;
    Ok(datum)
}

pub(super) fn evaluate_sum_for_rows(
    session: &mut Session<'_>,
    rows: &[&WorkingRow],
    outer: &[ScopeRecord],
    input: &dol_core::plan::LogicalExpr,
) -> Result<Datum> {
    let mut concrete = Vec::new();
    for row in rows {
        session.check_timeout()?;
        if let Datum::Value(value) = session.evaluate_expr(input, row, outer)? {
            concrete.push(value);
        }
    }
    if concrete.is_empty() {
        return Ok(Datum::Null);
    }

    let Some(sum) = sum_exact_values(&concrete)? else {
        return Ok(Datum::Null);
    };
    let datum = Datum::Value(sum);
    validate_datum(input.type_def(), Presence::Optional, &datum)?;
    Ok(datum)
}

fn evaluate_extreme(
    session: &mut Session<'_>,
    rows: &[&WorkingRow],
    outer: &[ScopeRecord],
    input: &dol_core::plan::LogicalExpr,
    maximum: bool,
) -> Result<Datum> {
    let mut selected: Option<Datum> = None;
    for row in rows {
        session.check_timeout()?;
        let candidate = session.evaluate_expr(input, row, outer)?;
        if !matches!(candidate, Datum::Value(_)) {
            continue;
        }
        let replace = match &selected {
            None => true,
            Some(current) => {
                let ordering = compare_datums(&candidate, current).ok_or_else(|| {
                    Diagnostic::error(
                        "MEMORY-AGG-003",
                        "aggregate input did not provide DOL ordering semantics",
                    )
                })?;
                if maximum {
                    ordering.is_gt()
                } else {
                    ordering.is_lt()
                }
            }
        };
        if replace {
            selected = Some(candidate);
        }
    }
    Ok(selected.unwrap_or(Datum::Null))
}

fn required_input(aggregate: &LogicalAggregate) -> Result<&dol_core::plan::LogicalExpr> {
    aggregate.input().ok_or_else(|| {
        Diagnostic::error(
            "MEMORY-AGG-004",
            "aggregate operation requires an input expression",
        )
    })
}
