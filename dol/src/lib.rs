#![forbid(unsafe_code)]
//! Public facade for the DOL workspace.

extern crate self as dol;

pub mod core;
pub mod prelude;

#[cfg(feature = "engine")]
pub mod engine;

#[cfg(feature = "migrate")]
pub mod migrate;

#[cfg(feature = "wire")]
pub mod wire;

#[cfg(feature = "ml")]
pub mod ml;

pub use dol_core::{
    BoundParameter, Cardinality, DataSet, DataType, DataValue, DateExprExt, Delete,
    DurationExprExt, Expr, ExprSource, Field, Insert, InsertMany, InstantExprExt, IntoExpr,
    LocalDateTimeExprExt, Model, ModelBuilder, ModelDef, ModelSet, NullableDateExprExt,
    NullableDurationExprExt, NullableInstantExprExt, NullableLocalDateTimeExprExt,
    NullableStringExprExt, NumericExprExt, Parameter, Parameters, Pipeline, ProjectionSource,
    RecordMut, RecordView, RelationDef, RowsFrame, RuntimeField, SemanticBinding, SemanticFunction,
    SignedNumericExprExt, StringExprExt, Truth, TypeDef, TypeFingerprint, TypeKey, TypeParameter,
    TypeProperties, Update, Window, WindowFrameBound, WriteOutcome, bind_value, dense_rank, rank,
    row_number, window_sum, window_sum_present,
};
pub use dol_core::{count, count_present, max, max_present, min, min_present, sum, sum_present};

#[cfg(feature = "derive")]
pub use dol_macros::{Model, Projection};
