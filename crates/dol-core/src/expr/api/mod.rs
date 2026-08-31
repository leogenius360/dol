//! Type-owned symbolic APIs for Rust values supported by DOL.
//!
//! These traits make `Field<T>` and `Expr<T>` behave like symbolic `T` without
//! pretending that a field is an actual Rust value. The traits are implemented
//! for every [`ExprSource`](super::ExprSource) of the corresponding Rust type.

mod numeric;
mod temporal;
mod text;

pub use numeric::{NumericExprExt, SignedNumericExprExt};
pub use temporal::{
    DateExprExt, DurationExprExt, InstantExprExt, LocalDateTimeExprExt, NullableDateExprExt,
    NullableDurationExprExt, NullableInstantExprExt, NullableLocalDateTimeExprExt,
};
pub use text::{NullableStringExprExt, StringExprExt};
