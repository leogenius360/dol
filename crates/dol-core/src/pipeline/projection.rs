//! Typed scalar, tuple, and named-record projection support.
//!
//! Projection is a role of [`crate::Pipeline`], not a separate computation
//! abstraction. These supporting traits only let `.select(...)` preserve the
//! Rust result type while lowering one or more symbolic expressions into a
//! canonical semantic output shape.

use core::marker::PhantomData;
use std::sync::Arc;

use crate::expr::{Expr, ExprSource, ExpressionSpec, Parameter, ScopedField};
use crate::model::Field;
use crate::runtime::RuntimeField;
use crate::types::{DataType, RecordField, TypeDef};

use crate::plan::ProjectionKind;

/// Type-erased authoring projection consumed by pipeline planning.
///
/// This type is public only so derive-generated code can cross crate
/// boundaries. Applications should normally pass fields, expressions, tuples,
/// or `#[derive(dol::Projection)]` record helpers directly to
/// [`crate::Pipeline::select`].
#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct ProjectionSpec {
    pub(crate) kind: ProjectionKind,
    pub(crate) expressions: Box<[ExpressionSpec]>,
    pub(crate) names: Box<[Arc<str>]>,
    pub(crate) ty: TypeDef,
}

impl ProjectionSpec {
    fn value(expression: ExpressionSpec) -> Self {
        Self {
            kind: ProjectionKind::Value,
            ty: expression.ty.clone(),
            expressions: vec![expression].into_boxed_slice(),
            names: Box::new([]),
        }
    }

    fn tuple(expressions: Vec<ExpressionSpec>) -> Self {
        let ty = TypeDef::tuple(expressions.iter().map(|expression| expression.ty.clone()));
        Self {
            kind: ProjectionKind::Tuple,
            expressions: expressions.into_boxed_slice(),
            names: Box::new([]),
            ty,
        }
    }
}

/// A typed source accepted by [`crate::Pipeline::select`].
///
/// This is a supporting authoring contract, not a sixth primary DOL
/// abstraction. Normal scalar sources implement it directly; tuples preserve
/// ordered product typing and derive-generated record helpers preserve field
/// names.
pub trait ProjectionSource {
    /// Rust value produced by the projection.
    type Value;

    /// Converts this source into type-erased projection semantics.
    #[doc(hidden)]
    fn __projection_spec(&self) -> ProjectionSpec;
}

macro_rules! scalar_projection_source {
    ($source:ty) => {
        impl<T> ProjectionSource for $source {
            type Value = T;

            fn __projection_spec(&self) -> ProjectionSpec {
                ProjectionSpec::value(self.expression().spec())
            }
        }
    };
}

scalar_projection_source!(Expr<T>);
scalar_projection_source!(Field<T>);
scalar_projection_source!(ScopedField<T>);
scalar_projection_source!(RuntimeField<T>);
scalar_projection_source!(Parameter<T>);

impl<S> ProjectionSource for &S
where
    S: ProjectionSource + ?Sized,
{
    type Value = S::Value;

    fn __projection_spec(&self) -> ProjectionSpec {
        S::__projection_spec(*self)
    }
}

/// Ordered tuple of scalar symbolic sources used by tuple and record projection.
///
/// This contract is exposed only for `#[derive(dol::Projection)]` expansion.
#[doc(hidden)]
pub trait ProjectionTuple {
    /// Rust tuple containing the values produced by the symbolic sources.
    type Value;

    /// Collects the source expressions in tuple position order.
    #[doc(hidden)]
    fn __projection_expressions(&self) -> Vec<ExpressionSpec>;
}

macro_rules! projection_tuple {
    ($($name:ident : $index:tt),+ $(,)?) => {
        impl<$($name),+> ProjectionTuple for ($($name,)+)
        where
            $($name: ExprSource,)+
        {
            type Value = ($($name::Value,)+);

            fn __projection_expressions(&self) -> Vec<ExpressionSpec> {
                vec![$(self.$index.expression().spec()),+]
            }
        }

        impl<$($name),+> ProjectionSource for ($($name,)+)
        where
            $($name: ExprSource,)+
        {
            type Value = ($($name::Value,)+);

            fn __projection_spec(&self) -> ProjectionSpec {
                ProjectionSpec::tuple(self.__projection_expressions())
            }
        }
    };
}

projection_tuple!(A: 0);
projection_tuple!(A: 0, B: 1);
projection_tuple!(A: 0, B: 1, C: 2);
projection_tuple!(A: 0, B: 1, C: 2, D: 3);
projection_tuple!(A: 0, B: 1, C: 2, D: 3, E: 4);
projection_tuple!(A: 0, B: 1, C: 2, D: 3, E: 4, F: 5);
projection_tuple!(A: 0, B: 1, C: 2, D: 3, E: 4, F: 5, G: 6);
projection_tuple!(A: 0, B: 1, C: 2, D: 3, E: 4, F: 5, G: 6, H: 7);
projection_tuple!(A: 0, B: 1, C: 2, D: 3, E: 4, F: 5, G: 6, H: 7, I: 8);
projection_tuple!(A: 0, B: 1, C: 2, D: 3, E: 4, F: 5, G: 6, H: 7, I: 8, J: 9);
projection_tuple!(A: 0, B: 1, C: 2, D: 3, E: 4, F: 5, G: 6, H: 7, I: 8, J: 9, K: 10);
projection_tuple!(A: 0, B: 1, C: 2, D: 3, E: 4, F: 5, G: 6, H: 7, I: 8, J: 9, K: 10, L: 11);

/// Metadata implemented by `#[derive(dol::Projection)]` for a named record result.
///
/// Applications do not need to implement or name this trait directly.
#[doc(hidden)]
pub trait ProjectionRecord: DataType {
    /// Rust tuple matching the record's declaration-order field types.
    type Fields;

    /// Record fields in declaration order, before canonical name sorting.
    #[doc(hidden)]
    fn __projection_fields() -> Vec<RecordField>;
}

/// Named-record projection created by derive-generated `Type::project(...)`.
///
/// The helper exists only during authoring and disappears into the logical
/// `Project` node. It does not represent a separate query or transformation.
#[doc(hidden)]
pub struct RecordProjection<R> {
    spec: ProjectionSpec,
    _marker: PhantomData<fn() -> R>,
}

impl<R> RecordProjection<R>
where
    R: ProjectionRecord,
{
    /// Creates a named projection from declaration-order symbolic sources.
    #[doc(hidden)]
    #[must_use]
    pub fn new<P>(sources: P) -> Self
    where
        P: ProjectionTuple<Value = R::Fields>,
    {
        let expressions = sources.__projection_expressions();
        let fields = R::__projection_fields();
        let names = fields
            .iter()
            .map(|field| Arc::<str>::from(field.name()))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Self {
            spec: ProjectionSpec {
                kind: ProjectionKind::Record,
                expressions: expressions.into_boxed_slice(),
                names,
                ty: R::type_def(),
            },
            _marker: PhantomData,
        }
    }
}

impl<R> ProjectionSource for RecordProjection<R>
where
    R: ProjectionRecord,
{
    type Value = R;

    fn __projection_spec(&self) -> ProjectionSpec {
        self.spec.clone()
    }
}
