use core::cmp::Ordering;

use dol_core::diagnostic::{Diagnostic, Result};
use dol_core::plan::{JoinKind, LogicalExpr, SetOperator, SortDirection, SortExpr};
use dol_core::semantics::{Truth, compare_datums};
use dol_core::value::{Datum, Value};
use dol_engine::ExecutionRow;

use super::Session;
use super::row::{ScopeRecord, WorkingRow};

pub(super) fn execute_sort(
    session: &mut Session<'_>,
    input: &[WorkingRow],
    outer: &[ScopeRecord],
    keys: &[SortExpr],
) -> Result<Vec<WorkingRow>> {
    let mut keyed = Vec::with_capacity(input.len());
    for (position, row) in input.iter().enumerate() {
        session.check_timeout()?;
        let values = keys
            .iter()
            .map(|key| session.evaluate_expr(key.expression(), row, outer))
            .collect::<Result<Vec<_>>>()?;
        for value in &values {
            if compare_datums(value, value).is_none() {
                return Err(Diagnostic::error(
                    "MEMORY-SORT-001",
                    "sort key did not provide DOL ordering semantics",
                ));
            }
        }
        keyed.push(KeyedRow {
            position,
            values,
            row: row.clone(),
        });
    }
    keyed.sort_by(|left, right| compare_keys(left, right, keys));
    session.check_timeout()?;
    Ok(keyed.into_iter().map(|entry| entry.row).collect())
}

struct KeyedRow {
    position: usize,
    values: Vec<Datum>,
    row: WorkingRow,
}

fn compare_keys(left: &KeyedRow, right: &KeyedRow, keys: &[SortExpr]) -> Ordering {
    for ((left_value, right_value), key) in left.values.iter().zip(&right.values).zip(keys) {
        let ordering = compare_datums(left_value, right_value).unwrap_or(Ordering::Equal);
        let ordering = match key.direction() {
            SortDirection::Ascending => ordering,
            SortDirection::Descending => ordering.reverse(),
        };
        if !ordering.is_eq() {
            return ordering;
        }
    }
    left.position.cmp(&right.position)
}

pub(super) fn execute_distinct(
    session: &mut Session<'_>,
    input: &[WorkingRow],
) -> Result<Vec<WorkingRow>> {
    let mut rows = Vec::new();
    let mut bytes = 0_u64;
    for row in input {
        session.check_timeout()?;
        if !rows
            .iter()
            .any(|existing: &WorkingRow| existing.output == row.output)
        {
            session.push_stage_row(&mut rows, &mut bytes, row.clone())?;
        }
    }
    Ok(rows)
}

pub(super) fn execute_set(
    session: &mut Session<'_>,
    left: &[WorkingRow],
    right: &[WorkingRow],
    operator: SetOperator,
) -> Result<Vec<WorkingRow>> {
    let mut rows = Vec::new();
    let mut bytes = 0_u64;
    match operator {
        SetOperator::UnionAll => {
            for row in left.iter().chain(right) {
                session.push_stage_row(&mut rows, &mut bytes, row.clone())?;
            }
        }
        SetOperator::Union => {
            for row in left.iter().chain(right) {
                push_unique(session, &mut rows, &mut bytes, row)?;
            }
        }
        SetOperator::Intersect => {
            for row in left {
                session.check_timeout()?;
                if right.iter().any(|candidate| candidate.output == row.output) {
                    push_unique(session, &mut rows, &mut bytes, row)?;
                }
            }
        }
        SetOperator::Except => {
            for row in left {
                session.check_timeout()?;
                if !right.iter().any(|candidate| candidate.output == row.output) {
                    push_unique(session, &mut rows, &mut bytes, row)?;
                }
            }
        }
    }
    Ok(rows)
}

fn push_unique(
    session: &Session<'_>,
    rows: &mut Vec<WorkingRow>,
    bytes: &mut u64,
    candidate: &WorkingRow,
) -> Result<()> {
    session.check_timeout()?;
    if !rows.iter().any(|row| row.output == candidate.output) {
        session.push_stage_row(rows, bytes, candidate.clone())?;
    }
    Ok(())
}

pub(super) struct JoinRequest<'a> {
    pub(super) left: &'a [WorkingRow],
    pub(super) right: &'a [WorkingRow],
    pub(super) left_shape: &'a [std::sync::Arc<dol_core::model::ModelDef>],
    pub(super) right_shape: &'a [std::sync::Arc<dol_core::model::ModelDef>],
    pub(super) outer: &'a [ScopeRecord],
    pub(super) kind: JoinKind,
    pub(super) condition: Option<&'a LogicalExpr>,
}

pub(super) fn execute_join(
    session: &mut Session<'_>,
    request: JoinRequest<'_>,
) -> Result<Vec<WorkingRow>> {
    if request.kind == JoinKind::Cross && request.condition.is_some() {
        return Err(Diagnostic::error(
            "MEMORY-JOIN-001",
            "cross join unexpectedly retained a join condition",
        ));
    }
    if request.kind != JoinKind::Cross && request.condition.is_none() {
        return Err(Diagnostic::error(
            "MEMORY-JOIN-002",
            "conditional join is missing its truth condition",
        ));
    }

    if request.kind == JoinKind::Cross {
        session.preflight_product(request.left.len(), request.right.len())?;
    }

    let mut output = Vec::new();
    let mut output_bytes = 0_u64;
    let mut right_matched = vec![false; request.right.len()];
    for left in request.left {
        let mut left_matched = false;
        for (right_index, right) in request.right.iter().enumerate() {
            session.check_timeout()?;
            if pair_matches(
                session,
                PairRequest {
                    left,
                    right,
                    outer: request.outer,
                    kind: request.kind,
                    condition: request.condition,
                },
            )? {
                left_matched = true;
                if let Some(matched) = right_matched.get_mut(right_index) {
                    *matched = true;
                }
                session.push_stage_row(
                    &mut output,
                    &mut output_bytes,
                    joined_row(left, right, request.kind),
                )?;
            }
        }
        if !left_matched && matches!(request.kind, JoinKind::Left | JoinKind::Full) {
            session.push_stage_row(
                &mut output,
                &mut output_bytes,
                unmatched_left(left, request.right_shape, request.kind),
            )?;
        }
    }

    if matches!(request.kind, JoinKind::Right | JoinKind::Full) {
        for (index, right) in request.right.iter().enumerate() {
            session.check_timeout()?;
            if !right_matched.get(index).copied().unwrap_or(false) {
                session.push_stage_row(
                    &mut output,
                    &mut output_bytes,
                    unmatched_right(right, request.left_shape, request.kind),
                )?;
            }
        }
    }
    Ok(output)
}

struct PairRequest<'a> {
    left: &'a WorkingRow,
    right: &'a WorkingRow,
    outer: &'a [ScopeRecord],
    kind: JoinKind,
    condition: Option<&'a LogicalExpr>,
}

fn pair_matches(session: &mut Session<'_>, request: PairRequest<'_>) -> Result<bool> {
    if request.kind == JoinKind::Cross {
        return Ok(true);
    }
    let condition = request
        .condition
        .ok_or_else(|| Diagnostic::error("MEMORY-JOIN-002", "conditional join has no condition"))?;
    let row = combined_row(request.left, request.right, JoinKind::Inner);
    match session.evaluate_expr(condition, &row, request.outer)? {
        Datum::Value(Value::Truth(Truth::True)) => Ok(true),
        Datum::Value(Value::Truth(Truth::False | Truth::Unknown)) => Ok(false),
        _ => Err(Diagnostic::error(
            "MEMORY-JOIN-003",
            "join condition did not evaluate to DOL truth",
        )),
    }
}

fn joined_row(left: &WorkingRow, right: &WorkingRow, kind: JoinKind) -> WorkingRow {
    combined_row(left, right, kind)
}

fn combined_row(left: &WorkingRow, right: &WorkingRow, kind: JoinKind) -> WorkingRow {
    let mut scopes = Vec::with_capacity(left.scopes.len() + right.scopes.len());
    scopes.extend_from_slice(&left.scopes);
    scopes.extend_from_slice(&right.scopes);
    WorkingRow {
        output: ExecutionRow::Product(
            vec![
                wrap_present(left.output.clone(), left_nullable(kind)),
                wrap_present(right.output.clone(), right_nullable(kind)),
            ]
            .into_boxed_slice(),
        ),
        scopes,
    }
}

fn unmatched_left(
    left: &WorkingRow,
    right_shape: &[std::sync::Arc<dol_core::model::ModelDef>],
    kind: JoinKind,
) -> WorkingRow {
    let mut scopes = left.scopes.clone();
    scopes.extend(right_shape.iter().cloned().map(ScopeRecord::null));
    WorkingRow {
        output: ExecutionRow::Product(
            vec![
                wrap_present(left.output.clone(), left_nullable(kind)),
                ExecutionRow::Nullable(None),
            ]
            .into_boxed_slice(),
        ),
        scopes,
    }
}

fn unmatched_right(
    right: &WorkingRow,
    left_shape: &[std::sync::Arc<dol_core::model::ModelDef>],
    kind: JoinKind,
) -> WorkingRow {
    let mut scopes = left_shape
        .iter()
        .cloned()
        .map(ScopeRecord::null)
        .collect::<Vec<_>>();
    scopes.extend_from_slice(&right.scopes);
    WorkingRow {
        output: ExecutionRow::Product(
            vec![
                ExecutionRow::Nullable(None),
                wrap_present(right.output.clone(), right_nullable(kind)),
            ]
            .into_boxed_slice(),
        ),
        scopes,
    }
}

fn wrap_present(output: ExecutionRow, nullable: bool) -> ExecutionRow {
    if nullable {
        ExecutionRow::Nullable(Some(Box::new(output)))
    } else {
        output
    }
}

const fn left_nullable(kind: JoinKind) -> bool {
    matches!(kind, JoinKind::Right | JoinKind::Full)
}

const fn right_nullable(kind: JoinKind) -> bool {
    matches!(kind, JoinKind::Left | JoinKind::Full)
}
