#![forbid(unsafe_code)]
//! Public facade for the DOL workspace.

extern crate self as dol;

/// Core DOL language re-exports.
pub mod core {
    pub use dol_core::*;
}

/// Common imports for DOL applications.
pub mod prelude {
    pub use dol_core::prelude::*;

    #[cfg(feature = "derive")]
    pub use dol_macros::{Model, Projection};

    #[cfg(feature = "engine")]
    pub use dol_engine::prelude::*;
}

#[cfg(feature = "engine")]
/// Engine SPI re-exports.
pub mod engine {
    pub use dol_engine::*;
}

#[cfg(feature = "migrate")]
/// Migration subsystem re-exports.
pub mod migrate {
    pub use dol_migrate::*;
}

#[cfg(feature = "wire")]
/// Bounded wire subsystem re-exports.
pub mod wire {
    pub use dol_wire::*;
}

#[cfg(feature = "ml")]
/// AI/ML operation re-exports.
pub mod ml {
    pub use dol_ml::*;
}

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
