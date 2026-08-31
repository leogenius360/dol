//! Storage-independent grouping and aggregate descriptors.
//!
//! Aggregates are supporting descriptors consumed by [`crate::Pipeline`]; they
//! are not a parallel computation abstraction. Global reduction is authored
//! with `Pipeline::aggregate`, while grouping plus reduction is one complete
//! `Pipeline::aggregate_by` operation. Grouping without reduction is already
//! expressible as `select(...).distinct()` and therefore does not require a
//! transient grouped-pipeline type.

use core::marker::PhantomData;

use crate::expr::{ExprSource, ExpressionSpec};
use crate::types::{DataType, TypeDef};

/// Primitive aggregate operation represented by a logical plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AggregateKind {
    /// Number of input rows, including rows whose fields are null or missing.
    CountRows,
    /// Number of concrete input values, excluding null and missing.
    CountPresent,
    /// Checked exact-numeric sum over concrete values.
    Sum,
    /// Minimum concrete value under DOL's stable key ordering subset.
    Min,
    /// Maximum concrete value under DOL's stable key ordering subset.
    Max,
}

/// Whether a value aggregate was authored from a statically non-null or
/// nullable Rust source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum AggregateInputMode {
    NonNull,
    Present,
}

/// Type-erased aggregate authoring descriptor.
///
/// Public only because aggregate-selection trait signatures cross crate
/// boundaries. Applications should use [`count`], [`count_present`], [`sum`],
/// [`sum_present`], [`min`], [`min_present`], [`max`], and [`max_present`].
#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct AggregateSpec {
    pub(crate) kind: AggregateKind,
    pub(crate) input: Option<ExpressionSpec>,
    pub(crate) input_mode: AggregateInputMode,
    pub(crate) ty: TypeDef,
}

/// One typed aggregate descriptor.
///
/// This helper is intentionally not part of DOL's primary conceptual core; it
/// only exists long enough for a pipeline to retain aggregate semantics.
#[doc(hidden)]
pub struct Aggregate<T> {
    spec: AggregateSpec,
    _marker: PhantomData<fn() -> T>,
}

impl<T> Clone for Aggregate<T> {
    fn clone(&self) -> Self {
        Self {
            spec: self.spec.clone(),
            _marker: PhantomData,
        }
    }
}

impl<T> core::fmt::Debug for Aggregate<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Aggregate")
            .field("kind", &self.spec.kind)
            .field("type", &self.spec.ty)
            .finish_non_exhaustive()
    }
}

/// Source accepted as one component of an aggregate selection.
pub trait AggregateSource {
    /// Rust result value produced by the aggregate.
    type Value;

    /// Returns the type-erased aggregate descriptor.
    #[doc(hidden)]
    fn __aggregate_spec(&self) -> AggregateSpec;
}

impl<T> AggregateSource for Aggregate<T> {
    type Value = T;

    fn __aggregate_spec(&self) -> AggregateSpec {
        self.spec.clone()
    }
}

/// Structural form of an aggregate selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AggregateSelectionKind {
    /// One aggregate result value.
    Value,
    /// Ordered heterogeneous tuple of aggregate result values.
    Tuple,
}

/// Type-erased aggregate selection consumed by pipeline planning.
#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct AggregateSelectionSpec {
    pub(crate) kind: AggregateSelectionKind,
    pub(crate) aggregates: Box<[AggregateSpec]>,
    pub(crate) ty: TypeDef,
}

/// One aggregate or an ordered tuple of aggregates accepted by pipeline reduction.
pub trait AggregateSelection {
    /// Rust value produced for each global result or group.
    type Value;

    /// Converts this selection into type-erased aggregate semantics.
    #[doc(hidden)]
    fn __aggregate_selection(&self) -> AggregateSelectionSpec;
}

impl<T> AggregateSelection for Aggregate<T> {
    type Value = T;

    fn __aggregate_selection(&self) -> AggregateSelectionSpec {
        AggregateSelectionSpec {
            kind: AggregateSelectionKind::Value,
            aggregates: vec![self.__aggregate_spec()].into_boxed_slice(),
            ty: self.spec.ty.clone(),
        }
    }
}

macro_rules! aggregate_tuple {
    ($($name:ident : $index:tt),+ $(,)?) => {
        impl<$($name),+> AggregateSelection for ($($name,)+)
        where
            $($name: AggregateSource,)+
        {
            type Value = ($($name::Value,)+);

            fn __aggregate_selection(&self) -> AggregateSelectionSpec {
                let aggregates = vec![$(self.$index.__aggregate_spec()),+];
                let ty = TypeDef::tuple(aggregates.iter().map(|aggregate| aggregate.ty.clone()));
                AggregateSelectionSpec {
                    kind: AggregateSelectionKind::Tuple,
                    aggregates: aggregates.into_boxed_slice(),
                    ty,
                }
            }
        }
    };
}

aggregate_tuple!(A: 0);
aggregate_tuple!(A: 0, B: 1);
aggregate_tuple!(A: 0, B: 1, C: 2);
aggregate_tuple!(A: 0, B: 1, C: 2, D: 3);
aggregate_tuple!(A: 0, B: 1, C: 2, D: 3, E: 4);
aggregate_tuple!(A: 0, B: 1, C: 2, D: 3, E: 4, F: 5);
aggregate_tuple!(A: 0, B: 1, C: 2, D: 3, E: 4, F: 5, G: 6);
aggregate_tuple!(A: 0, B: 1, C: 2, D: 3, E: 4, F: 5, G: 6, H: 7);
aggregate_tuple!(A: 0, B: 1, C: 2, D: 3, E: 4, F: 5, G: 6, H: 7, I: 8);
aggregate_tuple!(A: 0, B: 1, C: 2, D: 3, E: 4, F: 5, G: 6, H: 7, I: 8, J: 9);
aggregate_tuple!(A: 0, B: 1, C: 2, D: 3, E: 4, F: 5, G: 6, H: 7, I: 8, J: 9, K: 10);
aggregate_tuple!(A: 0, B: 1, C: 2, D: 3, E: 4, F: 5, G: 6, H: 7, I: 8, J: 9, K: 10, L: 11);

/// Counts input rows. The global aggregate of an empty pipeline is `0`.
#[must_use]
pub fn count() -> Aggregate<u64> {
    Aggregate {
        spec: AggregateSpec {
            kind: AggregateKind::CountRows,
            input: None,
            input_mode: AggregateInputMode::NonNull,
            ty: u64::type_def(),
        },
        _marker: PhantomData,
    }
}

/// Counts concrete values of `source`, excluding both null and missing.
#[must_use]
pub fn count_present<S>(source: S) -> Aggregate<u64>
where
    S: ExprSource,
{
    let expression = source.into_expression();
    Aggregate {
        spec: AggregateSpec {
            kind: AggregateKind::CountPresent,
            input: Some(expression.spec()),
            input_mode: AggregateInputMode::Present,
            ty: u64::type_def(),
        },
        _marker: PhantomData,
    }
}

fn present_value_aggregate<T, S>(source: S, kind: AggregateKind) -> Aggregate<Option<T>>
where
    S: ExprSource<Value = Option<T>>,
{
    let expression = source.into_expression();
    let ty = expression.type_def().clone().nullable();
    Aggregate {
        spec: AggregateSpec {
            kind,
            input: Some(expression.spec()),
            input_mode: AggregateInputMode::Present,
            ty,
        },
        _marker: PhantomData,
    }
}

/// Sums a statically non-null exact-numeric source using checked DOL arithmetic.
///
/// Missing values are ignored. The result is `None` when no concrete value is
/// available. Nullable sources must use [`sum_present`] so the Rust output type
/// does not acquire a meaningless nested `Option`.
#[must_use]
pub fn sum<S>(source: S) -> Aggregate<Option<S::Value>>
where
    S: ExprSource,
{
    let expression = source.into_expression();
    let ty = expression.type_def().clone().nullable();
    Aggregate {
        spec: AggregateSpec {
            kind: AggregateKind::Sum,
            input: Some(expression.spec()),
            input_mode: AggregateInputMode::NonNull,
            ty,
        },
        _marker: PhantomData,
    }
}

/// Sums concrete values from a nullable exact-numeric source, ignoring null and missing.
#[must_use]
pub fn sum_present<T, S>(source: S) -> Aggregate<Option<T>>
where
    S: ExprSource<Value = Option<T>>,
{
    present_value_aggregate(source, AggregateKind::Sum)
}

/// Returns the minimum of a statically non-null, stably ordered source.
///
/// Missing values are ignored and no concrete value produces `None`.
#[must_use]
pub fn min<S>(source: S) -> Aggregate<Option<S::Value>>
where
    S: ExprSource,
{
    let expression = source.into_expression();
    let ty = expression.type_def().clone().nullable();
    Aggregate {
        spec: AggregateSpec {
            kind: AggregateKind::Min,
            input: Some(expression.spec()),
            input_mode: AggregateInputMode::NonNull,
            ty,
        },
        _marker: PhantomData,
    }
}

/// Returns the minimum concrete value of a nullable, stably ordered source.
#[must_use]
pub fn min_present<T, S>(source: S) -> Aggregate<Option<T>>
where
    S: ExprSource<Value = Option<T>>,
{
    present_value_aggregate(source, AggregateKind::Min)
}

/// Returns the maximum of a statically non-null, stably ordered source.
///
/// Missing values are ignored and no concrete value produces `None`.
#[must_use]
pub fn max<S>(source: S) -> Aggregate<Option<S::Value>>
where
    S: ExprSource,
{
    let expression = source.into_expression();
    let ty = expression.type_def().clone().nullable();
    Aggregate {
        spec: AggregateSpec {
            kind: AggregateKind::Max,
            input: Some(expression.spec()),
            input_mode: AggregateInputMode::NonNull,
            ty,
        },
        _marker: PhantomData,
    }
}

/// Returns the maximum concrete value of a nullable, stably ordered source.
#[must_use]
pub fn max_present<T, S>(source: S) -> Aggregate<Option<T>>
where
    S: ExprSource<Value = Option<T>>,
{
    present_value_aggregate(source, AggregateKind::Max)
}
