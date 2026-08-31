//! Engine-facing read-only logical form of first-class write operations.

use crate::diagnostic::Result;
use crate::fingerprint::Fingerprint;
use crate::model::{FieldKey, FieldSlot, Model, ModelDef, RecordView};
use crate::plan::LogicalExpr;
use crate::runtime::DynRow;

use super::{AllAcknowledged, Delete, Insert, InsertMany, Scoped, Update};

/// Semantic write operation kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WriteKind {
    /// One inserted model row.
    Insert,
    /// Multiple inserted model rows.
    InsertMany,
    /// Simultaneous field assignment over selected rows.
    Update,
    /// Row deletion.
    Delete,
}

/// Destructive-operation scope retained by the logical write form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WriteScope {
    /// Rows are selected by one normalized truth expression.
    Filtered,
    /// The author explicitly acknowledged every row.
    All,
}

/// One prepared simultaneous update assignment.
#[derive(Debug, Clone)]
pub struct LogicalAssignment {
    field: FieldSlot,
    key: FieldKey,
    expression: LogicalExpr,
}

impl LogicalAssignment {
    /// Dense field slot updated by this assignment.
    #[must_use]
    pub const fn field(&self) -> FieldSlot {
        self.field
    }

    /// Stable semantic field key.
    #[must_use]
    pub const fn key(&self) -> &FieldKey {
        &self.key
    }

    /// Prepared right-hand expression evaluated against the pre-update row.
    #[must_use]
    pub const fn expression(&self) -> &LogicalExpr {
        &self.expression
    }
}

/// Read-only logical payload for one inserted row.
#[derive(Debug, Clone)]
pub struct LogicalInsert {
    model: Box<ModelDef>,
    row: DynRow,
    fingerprint: Fingerprint,
}

impl LogicalInsert {
    /// Exact target model semantics.
    #[must_use]
    pub fn model(&self) -> &ModelDef {
        &self.model
    }

    /// Canonical inserted row.
    #[must_use]
    pub const fn row(&self) -> &DynRow {
        &self.row
    }

    /// Canonical semantic operation fingerprint.
    #[must_use]
    pub const fn fingerprint(&self) -> Fingerprint {
        self.fingerprint
    }
}

/// Read-only logical payload for a bulk insert.
#[derive(Debug, Clone)]
pub struct LogicalInsertMany {
    model: Box<ModelDef>,
    rows: Box<[DynRow]>,
    batch_rows: Option<usize>,
    fingerprint: Fingerprint,
}

impl LogicalInsertMany {
    /// Exact target model semantics.
    #[must_use]
    pub fn model(&self) -> &ModelDef {
        &self.model
    }

    /// Canonical inserted rows in authored order.
    #[must_use]
    pub fn rows(&self) -> &[DynRow] {
        &self.rows
    }

    /// Preferred physical batch size, excluded from semantic identity.
    #[must_use]
    pub const fn batch_rows(&self) -> Option<usize> {
        self.batch_rows
    }

    /// Canonical semantic operation fingerprint.
    #[must_use]
    pub const fn fingerprint(&self) -> Fingerprint {
        self.fingerprint
    }
}

/// Read-only logical payload for one simultaneous update.
#[derive(Debug, Clone)]
pub struct LogicalUpdate {
    model: Box<ModelDef>,
    assignments: Box<[LogicalAssignment]>,
    condition: Option<Box<LogicalExpr>>,
    scope: WriteScope,
    fingerprint: Fingerprint,
}

impl LogicalUpdate {
    /// Exact target model semantics.
    #[must_use]
    pub fn model(&self) -> &ModelDef {
        &self.model
    }

    /// Canonically field-key ordered simultaneous assignments.
    #[must_use]
    pub fn assignments(&self) -> &[LogicalAssignment] {
        &self.assignments
    }

    /// Prepared filter, absent only after explicit `.all()` acknowledgement.
    #[must_use]
    pub fn condition(&self) -> Option<&LogicalExpr> {
        self.condition.as_deref()
    }

    /// Explicit destructive scope state.
    #[must_use]
    pub const fn scope(&self) -> WriteScope {
        self.scope
    }

    /// Canonical semantic operation fingerprint.
    #[must_use]
    pub const fn fingerprint(&self) -> Fingerprint {
        self.fingerprint
    }
}

/// Read-only logical payload for one delete.
#[derive(Debug, Clone)]
pub struct LogicalDelete {
    model: Box<ModelDef>,
    condition: Option<Box<LogicalExpr>>,
    scope: WriteScope,
    fingerprint: Fingerprint,
}

impl LogicalDelete {
    /// Exact target model semantics.
    #[must_use]
    pub fn model(&self) -> &ModelDef {
        &self.model
    }

    /// Prepared filter, absent only after explicit `.all()` acknowledgement.
    #[must_use]
    pub fn condition(&self) -> Option<&LogicalExpr> {
        self.condition.as_deref()
    }

    /// Explicit destructive scope state.
    #[must_use]
    pub const fn scope(&self) -> WriteScope {
        self.scope
    }

    /// Canonical semantic operation fingerprint.
    #[must_use]
    pub const fn fingerprint(&self) -> Fingerprint {
        self.fingerprint
    }
}

/// Backend-independent validated write consumed read-only by execution engines.
///
/// Variant payloads have private fields so adapters can inspect validated semantics
/// but cannot forge a destructive scope or bypass typed write preparation.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum LogicalWrite {
    /// One concrete inserted row.
    Insert(LogicalInsert),
    /// Multiple concrete inserted rows.
    InsertMany(LogicalInsertMany),
    /// Simultaneous update.
    Update(LogicalUpdate),
    /// Row deletion.
    Delete(LogicalDelete),
}

impl LogicalWrite {
    /// Operation family.
    #[must_use]
    pub const fn kind(&self) -> WriteKind {
        match self {
            Self::Insert(_) => WriteKind::Insert,
            Self::InsertMany(_) => WriteKind::InsertMany,
            Self::Update(_) => WriteKind::Update,
            Self::Delete(_) => WriteKind::Delete,
        }
    }

    /// Exact model definition targeted by this write.
    #[must_use]
    pub fn model(&self) -> &ModelDef {
        match self {
            Self::Insert(write) => write.model(),
            Self::InsertMany(write) => write.model(),
            Self::Update(write) => write.model(),
            Self::Delete(write) => write.model(),
        }
    }

    /// Canonical semantic write fingerprint.
    #[must_use]
    pub const fn fingerprint(&self) -> Fingerprint {
        match self {
            Self::Insert(write) => write.fingerprint(),
            Self::InsertMany(write) => write.fingerprint(),
            Self::Update(write) => write.fingerprint(),
            Self::Delete(write) => write.fingerprint(),
        }
    }
}

/// Hidden bridge from typed write authoring to the engine-facing logical form.
#[doc(hidden)]
pub trait WriteSource {
    /// Validates and lowers this write without introducing physical engine details.
    fn logical_write(&self) -> Result<LogicalWrite>;
}

impl<M> WriteSource for Insert<M>
where
    M: Model + RecordView,
{
    fn logical_write(&self) -> Result<LogicalWrite> {
        Ok(LogicalWrite::Insert(LogicalInsert {
            model: Box::new(M::model_def()?.clone()),
            row: DynRow::from_record(self.value())?,
            fingerprint: self.fingerprint()?,
        }))
    }
}

impl<M> WriteSource for InsertMany<M>
where
    M: Model + RecordView,
{
    fn logical_write(&self) -> Result<LogicalWrite> {
        let rows = self
            .values()
            .iter()
            .map(|row| DynRow::from_record(row))
            .collect::<Result<Vec<_>>>()?
            .into_boxed_slice();
        Ok(LogicalWrite::InsertMany(LogicalInsertMany {
            model: Box::new(M::model_def()?.clone()),
            rows,
            batch_rows: self.batch_size(),
            fingerprint: self.fingerprint()?,
        }))
    }
}

fn logical_update<M, S>(operation: &Update<M, S>, scope: WriteScope) -> Result<LogicalWrite>
where
    M: Model,
    Update<M, S>: PreparedUpdateSource,
{
    let prepared = operation.prepared_update()?;
    let assignments = prepared
        .assignments
        .into_vec()
        .into_iter()
        .map(|assignment| LogicalAssignment {
            field: assignment.field,
            key: assignment.key,
            expression: LogicalExpr::from_prepared(assignment.expression),
        })
        .collect::<Vec<_>>()
        .into_boxed_slice();
    let condition = prepared
        .condition
        .map(LogicalExpr::from_prepared)
        .map(Box::new);
    Ok(LogicalWrite::Update(LogicalUpdate {
        model: Box::new(M::model_def()?.clone()),
        assignments,
        condition,
        scope,
        fingerprint: operation.prepared_fingerprint()?,
    }))
}

fn logical_delete<M, S>(operation: &Delete<M, S>, scope: WriteScope) -> Result<LogicalWrite>
where
    M: Model,
    Delete<M, S>: PreparedDeleteSource,
{
    let prepared = operation.prepared_delete()?;
    Ok(LogicalWrite::Delete(LogicalDelete {
        model: Box::new(M::model_def()?.clone()),
        condition: prepared
            .condition
            .map(LogicalExpr::from_prepared)
            .map(Box::new),
        scope,
        fingerprint: operation.prepared_fingerprint()?,
    }))
}

trait PreparedUpdateSource {
    fn prepared_update(&self) -> Result<super::PreparedUpdate>;
    fn prepared_fingerprint(&self) -> Result<Fingerprint>;
}

impl<M> PreparedUpdateSource for Update<M, Scoped>
where
    M: Model,
{
    fn prepared_update(&self) -> Result<super::PreparedUpdate> {
        self.prepare()
    }

    fn prepared_fingerprint(&self) -> Result<Fingerprint> {
        self.fingerprint()
    }
}

impl<M> PreparedUpdateSource for Update<M, AllAcknowledged>
where
    M: Model,
{
    fn prepared_update(&self) -> Result<super::PreparedUpdate> {
        self.prepare()
    }

    fn prepared_fingerprint(&self) -> Result<Fingerprint> {
        self.fingerprint()
    }
}

trait PreparedDeleteSource {
    fn prepared_delete(&self) -> Result<super::PreparedDelete>;
    fn prepared_fingerprint(&self) -> Result<Fingerprint>;
}

impl<M> PreparedDeleteSource for Delete<M, Scoped>
where
    M: Model,
{
    fn prepared_delete(&self) -> Result<super::PreparedDelete> {
        self.prepare()
    }

    fn prepared_fingerprint(&self) -> Result<Fingerprint> {
        self.fingerprint()
    }
}

impl<M> PreparedDeleteSource for Delete<M, AllAcknowledged>
where
    M: Model,
{
    fn prepared_delete(&self) -> Result<super::PreparedDelete> {
        self.prepare()
    }

    fn prepared_fingerprint(&self) -> Result<Fingerprint> {
        self.fingerprint()
    }
}

impl<M> WriteSource for Update<M, Scoped>
where
    M: Model,
{
    fn logical_write(&self) -> Result<LogicalWrite> {
        logical_update(self, WriteScope::Filtered)
    }
}

impl<M> WriteSource for Update<M, AllAcknowledged>
where
    M: Model,
{
    fn logical_write(&self) -> Result<LogicalWrite> {
        logical_update(self, WriteScope::All)
    }
}

impl<M> WriteSource for Delete<M, Scoped>
where
    M: Model,
{
    fn logical_write(&self) -> Result<LogicalWrite> {
        logical_delete(self, WriteScope::Filtered)
    }
}

impl<M> WriteSource for Delete<M, AllAcknowledged>
where
    M: Model,
{
    fn logical_write(&self) -> Result<LogicalWrite> {
        logical_delete(self, WriteScope::All)
    }
}
