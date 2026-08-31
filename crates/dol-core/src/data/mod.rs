//! Concrete, eager, materialized data and its normative local write semantics.

mod validate;
mod write;

use crate::diagnostic::Result;
use crate::expr::{EvalContext, Expr};
use crate::model::{Model, RecordView};
use crate::ops::WriteOutcome;
use crate::semantics::Truth;
use crate::value::{Datum, Value};

#[doc(hidden)]
pub use validate::validate_dynamic_dataset;
pub(crate) use validate::{fingerprint_record, validate_dataset, validate_record};
pub use write::DataSetWrite;

/// An eager, materialized collection of values.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DataSet<T> {
    pub(crate) values: Vec<T>,
}

impl<T> DataSet<T> {
    /// Creates a data set from an iterator without imposing model semantics.
    #[must_use]
    pub fn new(values: impl IntoIterator<Item = T>) -> Self {
        Self {
            values: values.into_iter().collect(),
        }
    }

    /// Returns the number of materialized values.
    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Returns whether the data set is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Borrows the materialized values.
    #[must_use]
    pub fn as_slice(&self) -> &[T] {
        &self.values
    }

    /// Iterates over materialized values.
    pub fn iter(&self) -> core::slice::Iter<'_, T> {
        self.values.iter()
    }

    /// Consumes the data set and returns its materialized values.
    #[must_use]
    pub fn into_vec(self) -> Vec<T> {
        self.values
    }
}

impl<M> DataSet<M>
where
    M: Model + RecordView,
{
    /// Creates and validates a materialized model data set.
    pub fn try_new(values: impl IntoIterator<Item = M>) -> Result<Self> {
        let data = Self::new(values);
        data.validate()?;
        Ok(data)
    }

    /// Validates record shape, identity, and unique constraints.
    pub fn validate(&self) -> Result<()> {
        validate_dataset(&self.values)
    }

    /// Eagerly retains rows for which the DOL truth expression is `True`.
    ///
    /// `False` and `Unknown` are both excluded, matching pipeline filter semantics.
    pub fn filter(self, condition: Expr<Truth>) -> Result<Self> {
        self.validate()?;
        let model = M::model_def()?;
        let prepared = condition.prepare_for(model)?;
        let mut retained = Vec::with_capacity(self.values.len());
        for row in self.values {
            if matches!(
                prepared.evaluate_datum(&EvalContext::single(&row))?,
                Datum::Value(Value::Truth(Truth::True))
            ) {
                retained.push(row);
            }
        }
        Ok(Self { values: retained })
    }

    /// Applies one first-class write operation using normative local eager semantics.
    pub fn apply<W>(&mut self, write: W) -> Result<WriteOutcome>
    where
        W: DataSetWrite<M>,
    {
        write.apply_to(self)
    }
}

impl<T> From<Vec<T>> for DataSet<T> {
    fn from(values: Vec<T>) -> Self {
        Self { values }
    }
}

impl<T> FromIterator<T> for DataSet<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Self::new(iter)
    }
}

impl<T> IntoIterator for DataSet<T> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        self.values.into_iter()
    }
}

impl<'a, T> IntoIterator for &'a DataSet<T> {
    type Item = &'a T;
    type IntoIter = core::slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.values.iter()
    }
}
