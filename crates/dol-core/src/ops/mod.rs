//! First-class data-changing operations.

mod logical;
mod prepare;

use core::marker::PhantomData;

use crate::expr::{Expr, ExpressionSpec, IntoExpr};
use crate::model::Field;
use crate::semantics::Truth;
use crate::types::TypeDef;

pub use logical::{
    LogicalAssignment, LogicalDelete, LogicalInsert, LogicalInsertMany, LogicalUpdate,
    LogicalWrite, WriteKind, WriteScope, WriteSource,
};
pub(crate) use prepare::{PreparedDelete, PreparedUpdate, validate_batch_size};

/// Marker for an operation that has not selected a scope.
#[derive(Debug, Clone, Copy, Default)]
pub struct Unscoped;

/// Marker for an operation constrained by a condition.
#[derive(Debug, Clone, Copy, Default)]
pub struct Scoped;

/// Marker for an operation that explicitly acknowledges all rows.
#[derive(Debug, Clone, Copy, Default)]
pub struct AllAcknowledged;

/// Portable outcome of one locally applied write.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WriteOutcome {
    affected_rows: usize,
}

impl WriteOutcome {
    pub(crate) const fn new(affected_rows: usize) -> Self {
        Self { affected_rows }
    }

    /// Creates a portable write outcome from an engine-reported affected-row count.
    #[doc(hidden)]
    #[must_use]
    pub const fn from_affected_rows(affected_rows: usize) -> Self {
        Self { affected_rows }
    }

    /// Number of rows inserted, selected for update, or deleted.
    #[must_use]
    pub const fn affected_rows(&self) -> usize {
        self.affected_rows
    }
}

/// Inserts one model value.
#[derive(Debug, Clone)]
pub struct Insert<M> {
    value: M,
}

impl<M> Insert<M> {
    /// Creates a single-value insert operation.
    #[must_use]
    pub fn new(value: M) -> Self {
        Self { value }
    }

    /// Returns the value to insert.
    #[must_use]
    pub fn value(&self) -> &M {
        &self.value
    }

    pub(crate) fn into_value(self) -> M {
        self.value
    }
}

/// Inserts many model values.
#[derive(Debug, Clone)]
pub struct InsertMany<M> {
    values: Vec<M>,
    batch_rows: Option<usize>,
}

impl<M> InsertMany<M> {
    /// Creates a bulk insert operation.
    #[must_use]
    pub fn new(values: impl IntoIterator<Item = M>) -> Self {
        Self {
            values: values.into_iter().collect(),
            batch_rows: None,
        }
    }

    /// Sets the preferred physical execution batch size.
    ///
    /// This hint does not participate in semantic identity. A zero size is
    /// rejected when the operation is validated or executed.
    #[must_use]
    pub fn batch_rows(mut self, batch_rows: usize) -> Self {
        self.batch_rows = Some(batch_rows);
        self
    }

    /// Returns the values to insert.
    #[must_use]
    pub fn values(&self) -> &[M] {
        &self.values
    }

    /// Returns the preferred physical execution batch size.
    #[must_use]
    pub const fn batch_size(&self) -> Option<usize> {
        self.batch_rows
    }

    pub(crate) fn into_parts(self) -> (Vec<M>, Option<usize>) {
        (self.values, self.batch_rows)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Assignment {
    pub(crate) model: &'static str,
    pub(crate) field: &'static str,
    pub(crate) ty: TypeDef,
    pub(crate) value: ExpressionSpec,
}

/// A typed update operation with compile-time scope state.
#[derive(Debug, Clone)]
pub struct Update<M, S> {
    assignments: Vec<Assignment>,
    condition: Option<ExpressionSpec>,
    _marker: PhantomData<fn() -> (M, S)>,
}

impl<M> Update<M, Unscoped> {
    /// Creates an unscoped update operation.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            assignments: Vec::new(),
            condition: None,
            _marker: PhantomData,
        }
    }

    /// Scopes the update with a truth-valued expression.
    #[must_use]
    pub fn filter(self, condition: Expr<Truth>) -> Update<M, Scoped> {
        Update {
            assignments: self.assignments,
            condition: Some(condition.spec().canonical_truth()),
            _marker: PhantomData,
        }
    }

    /// Explicitly acknowledges an update over all rows.
    #[must_use]
    pub fn all(self) -> Update<M, AllAcknowledged> {
        Update {
            assignments: self.assignments,
            condition: None,
            _marker: PhantomData,
        }
    }
}

impl<M> Update<M, Scoped> {
    /// Adds another conjunctive row constraint.
    #[must_use]
    pub fn filter(mut self, condition: Expr<Truth>) -> Self {
        let condition = condition.spec().canonical_truth();
        self.condition = Some(match self.condition.take() {
            Some(current) => current.and_truth(condition),
            None => condition,
        });
        self
    }
}

impl<M, S> Update<M, S> {
    /// Adds one typed simultaneous assignment.
    ///
    /// All right-hand expressions observe the row state from before this
    /// update. Assignment order therefore has no semantic meaning.
    #[must_use]
    pub fn set<T>(mut self, field: Field<T>, value: impl IntoExpr<T>) -> Self {
        let value = value.into_expr();
        self.assignments.push(Assignment {
            model: field.model(),
            field: field.key(),
            ty: field.type_def(),
            value: value.spec(),
        });
        self
    }

    /// Returns the number of authored assignments.
    #[must_use]
    pub fn assignment_count(&self) -> usize {
        self.assignments.len()
    }

    pub(crate) fn assignments(&self) -> &[Assignment] {
        &self.assignments
    }

    pub(crate) fn condition(&self) -> Option<&ExpressionSpec> {
        self.condition.as_ref()
    }
}

impl<M> Default for Update<M, Unscoped> {
    fn default() -> Self {
        Self::new()
    }
}

/// A typed delete operation with compile-time scope state.
#[derive(Debug, Clone)]
pub struct Delete<M, S> {
    condition: Option<ExpressionSpec>,
    _marker: PhantomData<fn() -> (M, S)>,
}

impl<M> Delete<M, Unscoped> {
    /// Creates an unscoped delete operation.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            condition: None,
            _marker: PhantomData,
        }
    }

    /// Scopes the delete with a truth-valued expression.
    #[must_use]
    pub fn filter(self, condition: Expr<Truth>) -> Delete<M, Scoped> {
        Delete {
            condition: Some(condition.spec().canonical_truth()),
            _marker: PhantomData,
        }
    }

    /// Explicitly acknowledges deleting all rows.
    #[must_use]
    pub fn all(self) -> Delete<M, AllAcknowledged> {
        Delete {
            condition: None,
            _marker: PhantomData,
        }
    }
}

impl<M> Delete<M, Scoped> {
    /// Adds another conjunctive row constraint.
    #[must_use]
    pub fn filter(mut self, condition: Expr<Truth>) -> Self {
        let condition = condition.spec().canonical_truth();
        self.condition = Some(match self.condition.take() {
            Some(current) => current.and_truth(condition),
            None => condition,
        });
        self
    }
}

impl<M, S> Delete<M, S> {
    pub(crate) fn condition(&self) -> Option<&ExpressionSpec> {
        self.condition.as_ref()
    }
}

impl<M> Default for Delete<M, Unscoped> {
    fn default() -> Self {
        Self::new()
    }
}
