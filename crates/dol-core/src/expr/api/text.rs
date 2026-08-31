use crate::semantics::Truth;

use super::super::author::{Expr, ExprSource, IntoExpr};
use super::super::call::{builtin_call1, builtin_call2};
use super::super::function::BuiltinFunction;

/// Rust-string-compatible symbolic methods for non-null `String` expressions.
pub trait StringExprExt: ExprSource<Value = String> + Sized {
    /// Mirrors `str::to_lowercase` using DOL's pinned Unicode semantics.
    #[must_use]
    fn to_lowercase(&self) -> Expr<String> {
        builtin_call1(BuiltinFunction::TextLowercase, self)
    }

    /// Mirrors `str::to_uppercase` using DOL's pinned Unicode semantics.
    #[must_use]
    fn to_uppercase(&self) -> Expr<String> {
        builtin_call1(BuiltinFunction::TextUppercase, self)
    }

    /// Mirrors `str::trim` and returns owned symbolic text.
    #[must_use]
    fn trim(&self) -> Expr<String> {
        builtin_call1(BuiltinFunction::TextTrim, self)
    }

    /// Mirrors `str::trim_start` and returns owned symbolic text.
    #[must_use]
    fn trim_start(&self) -> Expr<String> {
        builtin_call1(BuiltinFunction::TextTrimStart, self)
    }

    /// Mirrors `str::trim_end` and returns owned symbolic text.
    #[must_use]
    fn trim_end(&self) -> Expr<String> {
        builtin_call1(BuiltinFunction::TextTrimEnd, self)
    }

    /// Mirrors exact string substring containment.
    #[must_use]
    fn contains(&self, other: impl IntoExpr<String>) -> Expr<Truth> {
        builtin_call2(BuiltinFunction::TextContains, self, other)
    }

    /// Mirrors exact string prefix matching.
    #[must_use]
    fn starts_with(&self, other: impl IntoExpr<String>) -> Expr<Truth> {
        builtin_call2(BuiltinFunction::TextStartsWith, self, other)
    }

    /// Mirrors `str::ends_with` for exact string suffix matching.
    #[must_use]
    fn ends_with(&self, other: impl IntoExpr<String>) -> Expr<Truth> {
        builtin_call2(BuiltinFunction::TextEndsWith, self, other)
    }

    /// Mirrors Rust string byte length with a portable `u64` result domain.
    #[must_use]
    fn len(&self) -> Expr<u64> {
        builtin_call1(BuiltinFunction::TextByteLength, self)
    }

    /// Tests whether the UTF-8 string is empty.
    #[must_use]
    fn is_empty(&self) -> Expr<Truth> {
        self.len().eq(0_u64)
    }

    /// Counts Unicode scalar values rather than UTF-8 bytes.
    #[must_use]
    fn scalar_len(&self) -> Expr<u64> {
        builtin_call1(BuiltinFunction::TextScalarLength, self)
    }
}

impl<S> StringExprExt for S where S: ExprSource<Value = String> + Sized {}

/// Rust-string-compatible symbolic methods for nullable `String` expressions.
pub trait NullableStringExprExt: ExprSource<Value = Option<String>> + Sized {
    /// Lowercases text while preserving null/missing state.
    #[must_use]
    fn to_lowercase(&self) -> Expr<Option<String>> {
        builtin_call1(BuiltinFunction::TextLowercase, self)
    }

    /// Uppercases text while preserving null/missing state.
    #[must_use]
    fn to_uppercase(&self) -> Expr<Option<String>> {
        builtin_call1(BuiltinFunction::TextUppercase, self)
    }

    /// Trims text while preserving null/missing state.
    #[must_use]
    fn trim(&self) -> Expr<Option<String>> {
        builtin_call1(BuiltinFunction::TextTrim, self)
    }

    /// Trims leading whitespace while preserving null/missing state.
    #[must_use]
    fn trim_start(&self) -> Expr<Option<String>> {
        builtin_call1(BuiltinFunction::TextTrimStart, self)
    }

    /// Trims trailing whitespace while preserving null/missing state.
    #[must_use]
    fn trim_end(&self) -> Expr<Option<String>> {
        builtin_call1(BuiltinFunction::TextTrimEnd, self)
    }

    /// Tests substring containment with three-valued null/missing semantics.
    #[must_use]
    fn contains(&self, other: impl IntoExpr<Option<String>>) -> Expr<Truth> {
        builtin_call2(BuiltinFunction::TextContains, self, other)
    }

    /// Tests prefix matching with three-valued null/missing semantics.
    #[must_use]
    fn starts_with(&self, other: impl IntoExpr<Option<String>>) -> Expr<Truth> {
        builtin_call2(BuiltinFunction::TextStartsWith, self, other)
    }

    /// Tests suffix matching with three-valued null/missing semantics.
    #[must_use]
    fn ends_with(&self, other: impl IntoExpr<Option<String>>) -> Expr<Truth> {
        builtin_call2(BuiltinFunction::TextEndsWith, self, other)
    }

    /// Returns UTF-8 byte length while preserving null/missing state.
    #[must_use]
    fn len(&self) -> Expr<Option<u64>> {
        builtin_call1(BuiltinFunction::TextByteLength, self)
    }

    /// Tests emptiness with three-valued null/missing semantics.
    #[must_use]
    fn is_empty(&self) -> Expr<Truth> {
        self.len().eq(0_u64)
    }

    /// Counts Unicode scalar values while preserving null/missing state.
    #[must_use]
    fn scalar_len(&self) -> Expr<Option<u64>> {
        builtin_call1(BuiltinFunction::TextScalarLength, self)
    }
}

impl<S> NullableStringExprExt for S where S: ExprSource<Value = Option<String>> + Sized {}
