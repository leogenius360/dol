use crate::diagnostic::{Diagnostic, Result};
use crate::expr::{EvalContext, PreparedExpression};
use crate::model::{Model, RecordMut, RecordView};
use crate::ops::{
    AllAcknowledged, Delete, Insert, InsertMany, PreparedDelete, PreparedUpdate, Scoped, Update,
    WriteOutcome,
};
use crate::semantics::Truth;
use crate::value::{Datum, Value};

use super::{DataSet, validate_dataset, validate_record};

/// Hidden execution bridge implemented only by DOL's first-class write operations.
///
/// Applications should call [`DataSet::apply`] rather than implement this trait.
#[doc(hidden)]
pub trait DataSetWrite<M> {
    /// Applies the operation atomically to local materialized data.
    fn apply_to(self, data: &mut DataSet<M>) -> Result<WriteOutcome>;
}

impl<M> DataSetWrite<M> for Insert<M>
where
    M: Model + RecordView,
{
    fn apply_to(self, data: &mut DataSet<M>) -> Result<WriteOutcome> {
        let value = self.into_value();
        validate_record(&value)?;
        data.values.push(value);
        if let Err(error) = validate_dataset(&data.values) {
            data.values.pop();
            return Err(error);
        }
        Ok(WriteOutcome::new(1))
    }
}

impl<M> DataSetWrite<M> for InsertMany<M>
where
    M: Model + RecordView,
{
    fn apply_to(self, data: &mut DataSet<M>) -> Result<WriteOutcome> {
        let (values, batch_size) = self.into_parts();
        crate::ops::validate_batch_size(batch_size)?;
        validate_dataset(&values)?;

        let original_len = data.values.len();
        let inserted = values.len();
        data.values.extend(values);
        if let Err(error) = validate_dataset(&data.values) {
            data.values.truncate(original_len);
            return Err(error);
        }
        Ok(WriteOutcome::new(inserted))
    }
}

impl<M> DataSetWrite<M> for Update<M, Scoped>
where
    M: Model + RecordMut + Clone,
{
    fn apply_to(self, data: &mut DataSet<M>) -> Result<WriteOutcome> {
        let prepared = self.prepare()?;
        apply_update(data, &prepared)
    }
}

impl<M> DataSetWrite<M> for Update<M, AllAcknowledged>
where
    M: Model + RecordMut + Clone,
{
    fn apply_to(self, data: &mut DataSet<M>) -> Result<WriteOutcome> {
        let prepared = self.prepare()?;
        apply_update(data, &prepared)
    }
}

impl<M> DataSetWrite<M> for Delete<M, Scoped>
where
    M: Model + RecordView,
{
    fn apply_to(self, data: &mut DataSet<M>) -> Result<WriteOutcome> {
        let prepared = self.prepare()?;
        apply_delete(data, &prepared)
    }
}

impl<M> DataSetWrite<M> for Delete<M, AllAcknowledged>
where
    M: Model + RecordView,
{
    fn apply_to(self, data: &mut DataSet<M>) -> Result<WriteOutcome> {
        let prepared = self.prepare()?;
        apply_delete(data, &prepared)
    }
}

fn apply_update<M>(data: &mut DataSet<M>, prepared: &PreparedUpdate) -> Result<WriteOutcome>
where
    M: Model + RecordMut + Clone,
{
    for row in &data.values {
        validate_record(row)?;
    }
    let mut candidate = data.values.clone();
    let mut affected = 0_usize;

    for row in &mut candidate {
        if !selected(prepared.condition.as_ref(), row)? {
            continue;
        }

        let values = prepared
            .assignments
            .iter()
            .map(|assignment| {
                assignment
                    .expression
                    .evaluate_datum(&EvalContext::single(&*row))
            })
            .collect::<Result<Vec<_>>>()?;

        for (assignment, value) in prepared.assignments.iter().zip(values) {
            row.set_field(assignment.field, value)?;
        }
        affected += 1;
    }

    validate_dataset(&candidate)?;
    data.values = candidate;
    Ok(WriteOutcome::new(affected))
}

fn apply_delete<M>(data: &mut DataSet<M>, prepared: &PreparedDelete) -> Result<WriteOutcome>
where
    M: Model + RecordView,
{
    for row in &data.values {
        validate_record(row)?;
    }
    let selected_rows = data
        .values
        .iter()
        .map(|row| selected(prepared.condition.as_ref(), row))
        .collect::<Result<Vec<_>>>()?;
    let affected = selected_rows.iter().filter(|selected| **selected).count();

    let mut index = 0_usize;
    data.values.retain(|_| {
        let keep = !selected_rows[index];
        index += 1;
        keep
    });
    Ok(WriteOutcome::new(affected))
}

fn selected(condition: Option<&PreparedExpression>, row: &dyn RecordView) -> Result<bool> {
    let Some(condition) = condition else {
        return Ok(true);
    };
    match condition.evaluate_datum(&EvalContext::single(row))? {
        Datum::Value(Value::Truth(Truth::True)) => Ok(true),
        Datum::Value(Value::Truth(Truth::False | Truth::Unknown)) => Ok(false),
        _ => Err(Diagnostic::error(
            "WRITE-FILTER-001",
            "prepared write filter did not evaluate to DOL truth",
        )),
    }
}
