use crate::data::{fingerprint_record, validate_dataset, validate_record};
use crate::diagnostic::{Diagnostic, Result};
use crate::expr::{BindContext, PreparedExpression, prepare_expression};
use crate::fingerprint::{CanonicalHasher, Fingerprint};
use crate::model::{FieldKey, FieldSlot, Model, RecordView};

use super::{AllAcknowledged, Delete, Insert, InsertMany, Scoped, Update};

#[derive(Debug, Clone)]
pub(crate) struct PreparedAssignment {
    pub(crate) field: FieldSlot,
    pub(crate) key: FieldKey,
    pub(crate) expression: PreparedExpression,
}

#[derive(Debug, Clone)]
pub(crate) struct PreparedUpdate {
    pub(crate) assignments: Box<[PreparedAssignment]>,
    pub(crate) condition: Option<PreparedExpression>,
}

#[derive(Debug, Clone)]
pub(crate) struct PreparedDelete {
    pub(crate) condition: Option<PreparedExpression>,
}

impl<M> Insert<M>
where
    M: Model + RecordView,
{
    /// Returns the canonical semantic identity of this insert.
    pub fn fingerprint(&self) -> Result<Fingerprint> {
        validate_record(self.value())?;
        let mut hasher = write_hasher::<M>(0)?;
        hasher.bytes(fingerprint_record(self.value())?.as_bytes());
        Ok(hasher.finish())
    }
}

impl<M> InsertMany<M>
where
    M: Model + RecordView,
{
    /// Returns the canonical semantic identity of this bulk insert.
    ///
    /// Physical batch size is deliberately excluded from semantic identity.
    pub fn fingerprint(&self) -> Result<Fingerprint> {
        validate_batch_size(self.batch_size())?;
        validate_dataset(self.values())?;
        let mut hasher = write_hasher::<M>(1)?;
        hasher.u64(self.values().len() as u64);
        for value in self.values() {
            hasher.bytes(fingerprint_record(value)?.as_bytes());
        }
        Ok(hasher.finish())
    }
}

impl<M> Update<M, Scoped>
where
    M: Model,
{
    /// Validates and fingerprints this scoped update.
    pub fn fingerprint(&self) -> Result<Fingerprint> {
        fingerprint_update::<M, _>(self, false)
    }

    pub(crate) fn prepare(&self) -> Result<PreparedUpdate> {
        prepare_update::<M, _>(self, false)
    }
}

impl<M> Update<M, AllAcknowledged>
where
    M: Model,
{
    /// Validates and fingerprints this explicitly all-row update.
    pub fn fingerprint(&self) -> Result<Fingerprint> {
        fingerprint_update::<M, _>(self, true)
    }

    pub(crate) fn prepare(&self) -> Result<PreparedUpdate> {
        prepare_update::<M, _>(self, true)
    }
}

impl<M> Delete<M, Scoped>
where
    M: Model,
{
    /// Validates and fingerprints this scoped delete.
    pub fn fingerprint(&self) -> Result<Fingerprint> {
        fingerprint_delete::<M, _>(self, false)
    }

    pub(crate) fn prepare(&self) -> Result<PreparedDelete> {
        prepare_delete::<M, _>(self, false)
    }
}

impl<M> Delete<M, AllAcknowledged>
where
    M: Model,
{
    /// Validates and fingerprints this explicitly all-row delete.
    pub fn fingerprint(&self) -> Result<Fingerprint> {
        fingerprint_delete::<M, _>(self, true)
    }

    pub(crate) fn prepare(&self) -> Result<PreparedDelete> {
        prepare_delete::<M, _>(self, true)
    }
}

pub(crate) fn validate_batch_size(batch_size: Option<usize>) -> Result<()> {
    if batch_size == Some(0) {
        return Err(Diagnostic::error(
            "WRITE-BATCH-001",
            "insert batch size must be greater than zero",
        ));
    }
    Ok(())
}

fn prepare_update<M, S>(operation: &Update<M, S>, all: bool) -> Result<PreparedUpdate>
where
    M: Model,
{
    if operation.assignments().is_empty() {
        return Err(Diagnostic::error(
            "WRITE-UPDATE-001",
            "update must assign at least one field",
        ));
    }

    let model = M::model_def()?;
    let context = BindContext::single(model);
    let mut assignments = operation.assignments().to_vec();
    assignments.sort_by(|left, right| left.field.cmp(right.field));

    let mut prepared = Vec::with_capacity(assignments.len());
    let mut previous = None;
    for assignment in assignments {
        if previous == Some(assignment.field) {
            return Err(Diagnostic::error(
                "WRITE-UPDATE-002",
                format!("field `{}` is assigned more than once", assignment.field),
            ));
        }
        previous = Some(assignment.field);

        if assignment.model != model.key().as_str() {
            return Err(Diagnostic::error(
                "WRITE-UPDATE-003",
                format!(
                    "field `{}` belongs to model `{}`, not `{}`",
                    assignment.field,
                    assignment.model,
                    model.key().as_str()
                ),
            ));
        }
        let field = model.field(assignment.field).ok_or_else(|| {
            Diagnostic::error(
                "WRITE-UPDATE-004",
                format!("model does not define field `{}`", assignment.field),
            )
        })?;
        if field.ty() != &assignment.ty {
            return Err(Diagnostic::error(
                "WRITE-UPDATE-005",
                format!(
                    "assignment type for field `{}` does not match the model",
                    assignment.field
                ),
            ));
        }
        let expression = prepare_expression(
            &assignment.value,
            &context,
            crate::limits::ExpressionLimits::default(),
        )?;
        if expression.ty != assignment.ty {
            return Err(Diagnostic::error(
                "WRITE-UPDATE-005",
                format!(
                    "assignment expression for field `{}` has the wrong semantic type",
                    assignment.field
                ),
            ));
        }
        prepared.push(PreparedAssignment {
            field: field.slot(),
            key: field.key().clone(),
            expression,
        });
    }

    let condition = prepare_condition(operation.condition(), &context, all)?;
    Ok(PreparedUpdate {
        assignments: prepared.into_boxed_slice(),
        condition,
    })
}

fn prepare_delete<M, S>(operation: &Delete<M, S>, all: bool) -> Result<PreparedDelete>
where
    M: Model,
{
    let model = M::model_def()?;
    let context = BindContext::single(model);
    Ok(PreparedDelete {
        condition: prepare_condition(operation.condition(), &context, all)?,
    })
}

fn prepare_condition(
    condition: Option<&crate::expr::ExpressionSpec>,
    context: &BindContext<'_>,
    all: bool,
) -> Result<Option<PreparedExpression>> {
    match (condition, all) {
        (Some(_), true) => Err(Diagnostic::error(
            "WRITE-SCOPE-001",
            "all-row acknowledgement cannot also carry a filter condition",
        )),
        (None, false) => Err(Diagnostic::error(
            "WRITE-SCOPE-002",
            "scoped destructive operation is missing its filter condition",
        )),
        (Some(condition), false) => Ok(Some(prepare_expression(
            condition,
            context,
            crate::limits::ExpressionLimits::default(),
        )?)),
        (None, true) => Ok(None),
    }
}

fn fingerprint_update<M, S>(operation: &Update<M, S>, all: bool) -> Result<Fingerprint>
where
    M: Model,
{
    let prepared = prepare_update::<M, S>(operation, all)?;
    let mut hasher = write_hasher::<M>(2)?;
    hash_scope(&mut hasher, prepared.condition.as_ref(), all);
    hasher.u64(prepared.assignments.len() as u64);
    for assignment in &prepared.assignments {
        hasher.str(assignment.key.as_str());
        hasher.bytes(assignment.expression.bound_fingerprint.as_bytes());
    }
    Ok(hasher.finish())
}

fn fingerprint_delete<M, S>(operation: &Delete<M, S>, all: bool) -> Result<Fingerprint>
where
    M: Model,
{
    let prepared = prepare_delete::<M, S>(operation, all)?;
    let mut hasher = write_hasher::<M>(3)?;
    hash_scope(&mut hasher, prepared.condition.as_ref(), all);
    Ok(hasher.finish())
}

fn write_hasher<M: Model>(kind: u8) -> Result<CanonicalHasher> {
    let mut hasher = CanonicalHasher::new(b"write/v1");
    hasher.u8(kind);
    hasher.bytes(M::model_def()?.fingerprint().as_bytes());
    Ok(hasher)
}

fn hash_scope(hasher: &mut CanonicalHasher, condition: Option<&PreparedExpression>, all: bool) {
    hasher.u8(u8::from(all));
    if let Some(condition) = condition {
        hasher.bytes(condition.bound_fingerprint.as_bytes());
    }
}
