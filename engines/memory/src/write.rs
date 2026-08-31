use std::time::Instant;

use dol_core::data::validate_dynamic_dataset;
use dol_core::diagnostic::{Diagnostic, Result};
use dol_core::expr::{EvalContext, Parameters};
use dol_core::model::{RecordMut, RecordView};
use dol_core::ops::{LogicalWrite, WriteOutcome};
use dol_core::semantics::Truth;
use dol_core::value::{Datum, Value};
use dol_engine::WriteLimits;

use crate::store::{Tables, register_definition};

pub(crate) fn apply_atomic_write<F>(
    tables: &mut Tables,
    write: &LogicalWrite,
    parameters: &Parameters,
    limits: &WriteLimits,
    validate: F,
) -> Result<WriteOutcome>
where
    F: FnOnce(&Tables) -> Result<()>,
{
    let key = write.model().key().clone();
    let previous = tables.get(&key).cloned();
    let outcome = match apply_write(tables, write, parameters, limits) {
        Ok(outcome) => outcome,
        Err(error) => {
            restore_table(tables, key, previous);
            return Err(error);
        }
    };
    if let Err(error) = validate(tables) {
        restore_table(tables, key, previous);
        return Err(error);
    }
    Ok(outcome)
}

fn restore_table(
    tables: &mut Tables,
    key: dol_core::model::ModelKey,
    previous: Option<crate::store::MemoryTable>,
) {
    match previous {
        Some(table) => {
            tables.insert(key, table);
        }
        None => {
            tables.remove(&key);
        }
    }
}

pub(crate) fn apply_write(
    tables: &mut Tables,
    write: &LogicalWrite,
    parameters: &Parameters,
    limits: &WriteLimits,
) -> Result<WriteOutcome> {
    let started = Instant::now();
    check_timeout(limits, started)?;
    match write {
        LogicalWrite::Insert(write) => {
            enforce_affected_limit(limits, 1)?;
            register_definition(tables, write.model().clone())?;
            let table = table_mut(tables, write.model())?;
            table.rows.push(write.row().clone());
            validate_dynamic_dataset(&table.model, &table.rows)?;
            check_timeout(limits, started)?;
            Ok(WriteOutcome::from_affected_rows(1))
        }
        LogicalWrite::InsertMany(write) => {
            enforce_affected_limit(limits, write.rows().len())?;
            register_definition(tables, write.model().clone())?;
            let table = table_mut(tables, write.model())?;
            table.rows.extend(write.rows().iter().cloned());
            validate_dynamic_dataset(&table.model, &table.rows)?;
            check_timeout(limits, started)?;
            Ok(WriteOutcome::from_affected_rows(write.rows().len()))
        }
        LogicalWrite::Update(write) => {
            let table = table_mut(tables, write.model())?;
            let mut affected = 0_usize;
            for row in &mut table.rows {
                check_timeout(limits, started)?;
                if !selected(write.condition(), row, parameters)? {
                    continue;
                }
                affected = affected.saturating_add(1);
                enforce_affected_limit(limits, affected)?;
                let values = {
                    let context = EvalContext::single(row).with_parameters(parameters);
                    write
                        .assignments()
                        .iter()
                        .map(|assignment| assignment.expression().evaluate_datum(&context))
                        .collect::<Result<Vec<_>>>()?
                };
                for (assignment, value) in write.assignments().iter().zip(values) {
                    row.set_field(assignment.field(), value)?;
                }
            }
            validate_dynamic_dataset(&table.model, &table.rows)?;
            check_timeout(limits, started)?;
            Ok(WriteOutcome::from_affected_rows(affected))
        }
        LogicalWrite::Delete(write) => {
            let table = table_mut(tables, write.model())?;
            let selected_rows = table
                .rows
                .iter()
                .map(|row| {
                    check_timeout(limits, started)?;
                    selected(write.condition(), row, parameters)
                })
                .collect::<Result<Vec<_>>>()?;
            let affected = selected_rows.iter().filter(|selected| **selected).count();
            enforce_affected_limit(limits, affected)?;
            let mut selected = selected_rows.into_iter();
            table.rows.retain(|_| !selected.next().unwrap_or(false));
            validate_dynamic_dataset(&table.model, &table.rows)?;
            check_timeout(limits, started)?;
            Ok(WriteOutcome::from_affected_rows(affected))
        }
        _ => Err(Diagnostic::error(
            "MEMORY-WRITE-005",
            "logical write kind is not recognized by this memory engine version",
        )),
    }
}

fn table_mut<'a>(
    tables: &'a mut Tables,
    model: &dol_core::model::ModelDef,
) -> Result<&'a mut crate::store::MemoryTable> {
    let table = tables.get_mut(model.key()).ok_or_else(|| {
        Diagnostic::error(
            "MEMORY-WRITE-001",
            format!("model `{}` is not registered", model.key().as_str()),
        )
    })?;
    if table.model.fingerprint() != model.fingerprint() {
        return Err(Diagnostic::error(
            "MEMORY-WRITE-002",
            "logical write model differs from the registered memory model",
        ));
    }
    Ok(table)
}

fn selected(
    condition: Option<&dol_core::plan::LogicalExpr>,
    row: &dyn RecordView,
    parameters: &Parameters,
) -> Result<bool> {
    let Some(condition) = condition else {
        return Ok(true);
    };
    if !condition.existential_dependencies().is_empty() || condition.outer_scope_count() != 0 {
        return Err(Diagnostic::error(
            "MEMORY-WRITE-003",
            "write filters cannot contain pipeline existential/correlation dependencies",
        ));
    }
    let context = EvalContext::single(row).with_parameters(parameters);
    match condition.evaluate_datum(&context)? {
        Datum::Value(Value::Truth(Truth::True)) => Ok(true),
        Datum::Value(Value::Truth(Truth::False | Truth::Unknown)) => Ok(false),
        _ => Err(Diagnostic::error(
            "MEMORY-WRITE-004",
            "logical write filter did not evaluate to DOL truth",
        )),
    }
}

fn enforce_affected_limit(limits: &WriteLimits, affected: usize) -> Result<()> {
    if let Some(max_rows) = limits.max_affected_rows
        && u64::try_from(affected).unwrap_or(u64::MAX) > max_rows
    {
        return Err(Diagnostic::error(
            "ENGINE-LIMIT-004",
            "memory write exceeded the configured affected-row limit",
        ));
    }
    Ok(())
}

fn check_timeout(limits: &WriteLimits, started: Instant) -> Result<()> {
    if let Some(timeout) = limits.timeout
        && started.elapsed() > timeout
    {
        return Err(Diagnostic::error(
            "ENGINE-LIMIT-006",
            "memory write exceeded the configured timeout",
        ));
    }
    Ok(())
}
