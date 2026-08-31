//! Typed authoring descriptors for deterministic window computation.
//!
//! A window descriptor is supporting metadata consumed immediately by
//! [`crate::Pipeline::window`]. It is not a parallel computation abstraction:
//! the resulting computation remains a [`crate::Pipeline`].

use core::marker::PhantomData;

use crate::expr::{ExprSource, ExpressionSpec};
use crate::plan::{RowsFrame, SortDirection, WindowFunctionKind};
use crate::types::{DataType, TypeDef};

use super::projection::{ProjectionSource, ProjectionSpec};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WindowInputMode {
    NonNull,
    Present,
}

#[derive(Debug, Clone)]
pub(crate) struct WindowOrderSpec {
    pub(crate) expression: ExpressionSpec,
    pub(crate) direction: SortDirection,
}

#[derive(Debug, Clone)]
pub(crate) struct WindowSpec {
    pub(crate) kind: WindowFunctionKind,
    pub(crate) input: Option<ExpressionSpec>,
    pub(crate) input_mode: WindowInputMode,
    pub(crate) partition: Option<ProjectionSpec>,
    pub(crate) order: Vec<WindowOrderSpec>,
    pub(crate) frame: Option<RowsFrame>,
    pub(crate) ty: TypeDef,
}

/// Typed window descriptor consumed by [`crate::Pipeline::window`].
///
/// Window descriptors exist only during authoring. They carry partition,
/// ordering, frame, and result-type semantics into one logical `Window` node.
pub struct Window<T> {
    pub(crate) spec: WindowSpec,
    _marker: PhantomData<fn() -> T>,
}

impl<T> Clone for Window<T> {
    fn clone(&self) -> Self {
        Self {
            spec: self.spec.clone(),
            _marker: PhantomData,
        }
    }
}

impl<T> core::fmt::Debug for Window<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Window")
            .field("kind", &self.spec.kind)
            .field("type", &self.spec.ty)
            .field("order_keys", &self.spec.order.len())
            .field("frame", &self.spec.frame)
            .finish_non_exhaustive()
    }
}

impl<T> Window<T> {
    /// Partitions input rows by one scalar, tuple, or named-record key.
    #[must_use]
    pub fn partition_by<P>(mut self, keys: P) -> Self
    where
        P: ProjectionSource,
    {
        self.spec.partition = Some(keys.__projection_spec());
        self
    }

    /// Adds one ascending deterministic window ordering key.
    #[must_use]
    pub fn order_by<S>(self, expression: S) -> Self
    where
        S: ExprSource,
    {
        self.order(expression, SortDirection::Ascending)
    }

    /// Adds one descending deterministic window ordering key.
    #[must_use]
    pub fn order_by_desc<S>(self, expression: S) -> Self
    where
        S: ExprSource,
    {
        self.order(expression, SortDirection::Descending)
    }

    /// Sets an explicit row-count frame for an aggregate-style window.
    ///
    /// Ranking windows reject frames because their semantics depend only on
    /// partition and peer ordering, not frame membership.
    #[must_use]
    pub fn rows(mut self, frame: RowsFrame) -> Self {
        self.spec.frame = Some(frame);
        self
    }

    fn order<S>(mut self, expression: S, direction: SortDirection) -> Self
    where
        S: ExprSource,
    {
        self.spec.order.push(WindowOrderSpec {
            expression: expression.expression().spec(),
            direction,
        });
        self
    }
}

fn ranking(kind: WindowFunctionKind) -> Window<u64> {
    Window {
        spec: WindowSpec {
            kind,
            input: None,
            input_mode: WindowInputMode::NonNull,
            partition: None,
            order: Vec::new(),
            frame: None,
            ty: u64::type_def(),
        },
        _marker: PhantomData,
    }
}

/// One-based deterministic row position within each ordered partition.
///
/// Planning requires at least one stable ordering key so physical engines
/// cannot choose different row numbers for the same semantic plan.
#[must_use]
pub fn row_number() -> Window<u64> {
    ranking(WindowFunctionKind::RowNumber)
}

/// One-based peer rank with gaps between peer groups.
#[must_use]
pub fn rank() -> Window<u64> {
    ranking(WindowFunctionKind::Rank)
}

/// One-based peer rank without gaps between peer groups.
#[must_use]
pub fn dense_rank() -> Window<u64> {
    ranking(WindowFunctionKind::DenseRank)
}

/// Checked exact-numeric sum over an explicit row-count frame.
///
/// The result is nullable because a selected frame can contain no concrete
/// values. Nullable sources must use [`window_sum_present`].
#[must_use]
pub fn window_sum<S>(source: S) -> Window<Option<S::Value>>
where
    S: ExprSource,
{
    let expression = source.into_expression();
    Window {
        spec: WindowSpec {
            kind: WindowFunctionKind::Sum,
            input: Some(expression.spec()),
            input_mode: WindowInputMode::NonNull,
            partition: None,
            order: Vec::new(),
            frame: None,
            ty: expression.type_def().clone().nullable(),
        },
        _marker: PhantomData,
    }
}

/// Checked exact-numeric sum of concrete nullable values over an explicit rows frame.
#[must_use]
pub fn window_sum_present<T, S>(source: S) -> Window<Option<T>>
where
    S: ExprSource<Value = Option<T>>,
{
    let expression = source.into_expression();
    Window {
        spec: WindowSpec {
            kind: WindowFunctionKind::Sum,
            input: Some(expression.spec()),
            input_mode: WindowInputMode::Present,
            partition: None,
            order: Vec::new(),
            frame: None,
            ty: expression.type_def().clone().nullable(),
        },
        _marker: PhantomData,
    }
}
