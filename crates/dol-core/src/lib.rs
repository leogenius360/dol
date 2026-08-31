#![forbid(unsafe_code)]
//! DOL's storage-independent semantic language and normative local foundation.

pub mod analytics;
pub mod binding;
pub mod data;
pub mod diagnostic;
pub mod error;
pub mod expr;
pub mod fingerprint;
pub mod limits;
pub mod model;
pub mod ops;
pub mod pipeline;
pub mod plan;
pub mod runtime;
pub mod semantics;
pub mod types;
pub mod value;

/// Common semantic-language imports.
pub mod prelude {
    pub use crate::analytics::{
        count, count_present, max, max_present, min, min_present, sum, sum_present,
    };
    pub use crate::binding::{SemanticBinding, bind_value};
    pub use crate::data::DataSet;
    pub use crate::expr::{
        BoundParameter, DateExprExt, DurationExprExt, Expr, ExprSource, InstantExprExt, IntoExpr,
        LocalDateTimeExprExt, NullableDateExprExt, NullableDurationExprExt, NullableInstantExprExt,
        NullableLocalDateTimeExprExt, NullableStringExprExt, NumericExprExt, Parameter, Parameters,
        SemanticFunction, SignedNumericExprExt, StringExprExt,
    };
    pub use crate::model::{
        Cardinality, Field, Model, ModelBuilder, ModelOperations, ModelSet, Presence, RecordMut,
        RecordView, RelationDef,
    };
    pub use crate::ops::{
        AllAcknowledged, Delete, Insert, InsertMany, Scoped, Unscoped, Update, WriteOutcome,
    };
    pub use crate::pipeline::{
        Pipeline, ProjectionSource, RowsFrame, Window, WindowFrameBound, dense_rank, rank,
        row_number, window_sum, window_sum_present,
    };
    pub use crate::runtime::RuntimeField;
    pub use crate::semantics::Truth;
    pub use crate::types::{
        DataType, Nullability, ScalarRepr, TypeDef, TypeFingerprint, TypeKey, TypeParameter,
        TypeProperties, TypeShape,
    };
    pub use crate::value::{Bytes, DataValue, Datum, DatumRef, RecordValueView, Value, ValueRef};
}

pub use analytics::{count, count_present, max, max_present, min, min_present, sum, sum_present};
pub use binding::{SemanticBinding, bind_value};
pub use data::DataSet;
pub use expr::{
    BoundParameter, DateExprExt, DurationExprExt, Expr, ExprSource, InstantExprExt, IntoExpr,
    LocalDateTimeExprExt, NullableDateExprExt, NullableDurationExprExt, NullableInstantExprExt,
    NullableLocalDateTimeExprExt, NullableStringExprExt, NumericExprExt, Parameter, Parameters,
    SemanticFunction, SignedNumericExprExt, StringExprExt,
};
pub use model::{
    Cardinality, Field, Model, ModelBuilder, ModelDef, ModelSet, RecordMut, RecordView, RelationDef,
};
pub use ops::{Delete, Insert, InsertMany, Update, WriteOutcome};
pub use pipeline::{
    Pipeline, ProjectionSource, RowsFrame, Window, WindowFrameBound, dense_rank, rank, row_number,
    window_sum, window_sum_present,
};
pub use runtime::RuntimeField;
pub use semantics::Truth;
pub use types::{DataType, TypeDef, TypeFingerprint, TypeKey, TypeParameter, TypeProperties};
pub use value::{DataValue, Datum, Value};
