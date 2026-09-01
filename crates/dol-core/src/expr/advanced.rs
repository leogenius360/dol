use crate::binding::Binding;
use crate::model::Field;
use crate::runtime::RuntimeField;
use crate::semantics::Truth;
use crate::types::DataType;

use super::author::{Expr, IntoExpr, IntoTypedExpr};
use super::node::{BinaryOp, ExprKind, ExprNode, UnaryOp};

/// Built-in numeric semantic types eligible for explicit lossless casts.
pub trait LosslessNumericCastType: DataType {}

/// Semantic types with DOL unary numeric negation.
pub trait NegatableType: DataType {}

/// Integral semantic types with DOL remainder semantics.
pub trait IntegralType: DataType {}

macro_rules! lossless_cast_type {
    ($($ty:ty),+ $(,)?) => {
        $(impl LosslessNumericCastType for $ty {})+
    };
}

macro_rules! negatable_type {
    ($($ty:ty),+ $(,)?) => {
        $(impl NegatableType for $ty {})+
    };
}

macro_rules! integral_type {
    ($($ty:ty),+ $(,)?) => {
        $(impl IntegralType for $ty {})+
    };
}

lossless_cast_type!(
    i8,
    i16,
    i32,
    i64,
    i128,
    u8,
    u16,
    u32,
    u64,
    u128,
    f32,
    f64,
    rust_decimal::Decimal,
);
negatable_type!(i8, i16, i32, i64, i128, f32, f64, rust_decimal::Decimal);
integral_type!(i8, i16, i32, i64, i128, u8, u16, u32, u64, u128);

impl<T: LosslessNumericCastType> LosslessNumericCastType for Option<T> {}
impl<T: NegatableType> NegatableType for Option<T> {}
impl<T: IntegralType> IntegralType for Option<T> {}

impl<T> Expr<T> {
    /// Null-safe inequality: always yields `True` or `False`, never `Unknown`.
    #[must_use]
    pub fn is_distinct_from(self, other: impl IntoExpr<T>) -> Expr<Truth> {
        binary_truth(BinaryOp::IsDistinctFrom, self, other.into_expr())
    }

    /// Null-safe equality: always yields `True` or `False`, never `Unknown`.
    #[must_use]
    pub fn is_not_distinct_from(self, other: impl IntoExpr<T>) -> Expr<Truth> {
        binary_truth(BinaryOp::IsNotDistinctFrom, self, other.into_expr())
    }

    /// Tests membership against a finite expression candidate collection.
    ///
    /// Candidate order is non-semantic and is canonicalized during normalization.
    #[must_use]
    pub fn is_in<I, R>(self, candidates: I) -> Expr<Truth>
    where
        I: IntoIterator<Item = R>,
        R: IntoExpr<T>,
    {
        membership(self, candidates, false)
    }

    /// Tests non-membership against a finite expression candidate collection.
    #[must_use]
    pub fn not_in<I, R>(self, candidates: I) -> Expr<Truth>
    where
        I: IntoIterator<Item = R>,
        R: IntoExpr<T>,
    {
        membership(self, candidates, true)
    }

    /// Inclusive range test, equivalent to `self >= lower AND self <= upper`.
    #[must_use]
    pub fn between(self, lower: impl IntoExpr<T>, upper: impl IntoExpr<T>) -> Expr<Truth> {
        self.clone().ge(lower).and(self.le(upper))
    }

    /// Inclusive inverse range test.
    #[must_use]
    pub fn not_between(self, lower: impl IntoExpr<T>, upper: impl IntoExpr<T>) -> Expr<Truth> {
        self.between(lower, upper).negate()
    }
}

impl<T> Expr<Option<T>> {
    /// Uses `fallback` when this expression is null or missing, producing non-null semantic `T`.
    #[must_use]
    pub fn coalesce(self, fallback: impl IntoExpr<T>) -> Expr<T> {
        let fallback = fallback.into_expr();
        let binding = fallback.binding;
        Expr::from_node(
            ExprNode {
                kind: ExprKind::Binary {
                    op: BinaryOp::Coalesce,
                    left: self.node,
                    right: fallback.node,
                },
                ty: binding.type_def(),
                fingerprint: std::sync::OnceLock::new(),
            },
            binding,
        )
    }

    /// Uses a nullable fallback while preserving nullable result semantics.
    #[must_use]
    pub fn coalesce_nullable(self, fallback: impl IntoExpr<Option<T>>) -> Self {
        let fallback = fallback.into_expr();
        binary_same(BinaryOp::Coalesce, self, fallback)
    }
}

impl Expr<Truth> {
    /// Chooses between two same-typed expressions.
    ///
    /// `Truth::True` selects `when_true`; `False` and `Unknown` select `when_false`,
    /// matching DOL searched-conditional semantics.
    #[must_use]
    pub fn if_else<B, F>(self, when_true: B, when_false: F) -> Expr<B::Output>
    where
        B: IntoTypedExpr,
        F: IntoExpr<B::Output>,
    {
        let when_true = when_true.into_typed_expr();
        let when_false = when_false.into_expr();
        let binding = when_true.binding;
        Expr::from_node(
            ExprNode {
                kind: ExprKind::Conditional {
                    condition: self.node,
                    when_true: when_true.node,
                    when_false: when_false.node,
                },
                ty: binding.type_def(),
                fingerprint: std::sync::OnceLock::new(),
            },
            binding,
        )
    }
}

impl<T> core::ops::Neg for Expr<T>
where
    T: NegatableType,
{
    type Output = Self;

    fn neg(self) -> Self::Output {
        let ty = self.node.ty.clone();
        let binding = self.binding;
        Expr::from_node(
            ExprNode {
                kind: ExprKind::Unary {
                    op: UnaryOp::Negate,
                    input: self.node,
                },
                ty,
                fingerprint: std::sync::OnceLock::new(),
            },
            binding,
        )
    }
}

impl<T, R> core::ops::Rem<R> for Expr<T>
where
    T: IntegralType,
    R: IntoExpr<T>,
{
    type Output = Self;

    fn rem(self, rhs: R) -> Self::Output {
        binary_same(BinaryOp::Rem, self, rhs.into_expr())
    }
}

impl<T> Field<T> {
    /// Null-safe inequality comparison.
    #[must_use]
    pub fn is_distinct_from(self, other: impl IntoExpr<T>) -> Expr<Truth> {
        self.expr().is_distinct_from(other)
    }

    /// Null-safe equality comparison.
    #[must_use]
    pub fn is_not_distinct_from(self, other: impl IntoExpr<T>) -> Expr<Truth> {
        self.expr().is_not_distinct_from(other)
    }

    /// Finite candidate-collection membership test.
    #[must_use]
    pub fn is_in<I, R>(self, candidates: I) -> Expr<Truth>
    where
        I: IntoIterator<Item = R>,
        R: IntoExpr<T>,
    {
        self.expr().is_in(candidates)
    }

    /// Finite candidate-collection non-membership test.
    #[must_use]
    pub fn not_in<I, R>(self, candidates: I) -> Expr<Truth>
    where
        I: IntoIterator<Item = R>,
        R: IntoExpr<T>,
    {
        self.expr().not_in(candidates)
    }
}

impl<T> RuntimeField<T> {
    /// Null-safe inequality comparison.
    #[must_use]
    pub fn is_distinct_from(&self, other: impl IntoExpr<T>) -> Expr<Truth> {
        self.expr().is_distinct_from(other)
    }

    /// Null-safe equality comparison.
    #[must_use]
    pub fn is_not_distinct_from(&self, other: impl IntoExpr<T>) -> Expr<Truth> {
        self.expr().is_not_distinct_from(other)
    }

    /// Finite candidate-collection membership test.
    #[must_use]
    pub fn is_in<I, R>(&self, candidates: I) -> Expr<Truth>
    where
        I: IntoIterator<Item = R>,
        R: IntoExpr<T>,
    {
        self.expr().is_in(candidates)
    }

    /// Finite candidate-collection non-membership test.
    #[must_use]
    pub fn not_in<I, R>(&self, candidates: I) -> Expr<Truth>
    where
        I: IntoIterator<Item = R>,
        R: IntoExpr<T>,
    {
        self.expr().not_in(candidates)
    }
}

impl<T, R> core::ops::Rem<R> for Field<T>
where
    T: IntegralType,
    R: IntoExpr<T>,
{
    type Output = Expr<T>;

    fn rem(self, rhs: R) -> Self::Output {
        self.expr() % rhs
    }
}

impl<T, R> core::ops::Rem<R> for RuntimeField<T>
where
    T: IntegralType,
    R: IntoExpr<T>,
{
    type Output = Expr<T>;

    fn rem(self, rhs: R) -> Self::Output {
        self.expr() % rhs
    }
}

impl<T, R> core::ops::Rem<R> for &RuntimeField<T>
where
    T: IntegralType,
    R: IntoExpr<T>,
{
    type Output = Expr<T>;

    fn rem(self, rhs: R) -> Self::Output {
        self.expr() % rhs
    }
}

impl<T> Field<T> {
    /// Inclusive range test.
    #[must_use]
    pub fn between(self, lower: impl IntoExpr<T>, upper: impl IntoExpr<T>) -> Expr<Truth> {
        self.expr().between(lower, upper)
    }

    /// Inclusive inverse range test.
    #[must_use]
    pub fn not_between(self, lower: impl IntoExpr<T>, upper: impl IntoExpr<T>) -> Expr<Truth> {
        self.expr().not_between(lower, upper)
    }
}

impl<T> RuntimeField<T> {
    /// Inclusive range test.
    #[must_use]
    pub fn between(&self, lower: impl IntoExpr<T>, upper: impl IntoExpr<T>) -> Expr<Truth> {
        self.expr().between(lower, upper)
    }

    /// Inclusive inverse range test.
    #[must_use]
    pub fn not_between(&self, lower: impl IntoExpr<T>, upper: impl IntoExpr<T>) -> Expr<Truth> {
        self.expr().not_between(lower, upper)
    }
}

impl<T> Field<Option<T>> {
    /// Uses `fallback` when this field is null or missing.
    #[must_use]
    pub fn coalesce(self, fallback: impl IntoExpr<T>) -> Expr<T> {
        self.expr().coalesce(fallback)
    }

    /// Uses a nullable fallback while preserving nullable semantics.
    #[must_use]
    pub fn coalesce_nullable(self, fallback: impl IntoExpr<Option<T>>) -> Expr<Option<T>> {
        self.expr().coalesce_nullable(fallback)
    }
}

impl<T> RuntimeField<Option<T>> {
    /// Uses `fallback` when this field is null or missing.
    #[must_use]
    pub fn coalesce(&self, fallback: impl IntoExpr<T>) -> Expr<T> {
        self.expr().coalesce(fallback)
    }

    /// Uses a nullable fallback while preserving nullable semantics.
    #[must_use]
    pub fn coalesce_nullable(&self, fallback: impl IntoExpr<Option<T>>) -> Expr<Option<T>> {
        self.expr().coalesce_nullable(fallback)
    }
}

impl<T> core::ops::Neg for Field<T>
where
    T: NegatableType,
{
    type Output = Expr<T>;

    fn neg(self) -> Self::Output {
        -self.expr()
    }
}

impl<T> core::ops::Neg for RuntimeField<T>
where
    T: NegatableType,
{
    type Output = Expr<T>;

    fn neg(self) -> Self::Output {
        -self.expr()
    }
}

impl<T> core::ops::Neg for &RuntimeField<T>
where
    T: NegatableType,
{
    type Output = Expr<T>;

    fn neg(self) -> Self::Output {
        -self.expr()
    }
}

fn membership<T, I, R>(input: Expr<T>, candidates: I, negate: bool) -> Expr<Truth>
where
    I: IntoIterator<Item = R>,
    R: IntoExpr<T>,
{
    let candidates = candidates
        .into_iter()
        .map(|candidate| candidate.into_expr().node)
        .collect();
    Expr::from_node(
        ExprNode {
            kind: ExprKind::Membership {
                input: input.node,
                candidates,
                negate,
            },
            ty: Truth::type_def(),
            fingerprint: std::sync::OnceLock::new(),
        },
        Binding::<Truth>::native(),
    )
}

fn binary_same<T>(op: BinaryOp, left: Expr<T>, right: Expr<T>) -> Expr<T> {
    let ty = left.node.ty.clone();
    let binding = left.binding;
    Expr::from_node(
        ExprNode {
            kind: ExprKind::Binary {
                op,
                left: left.node,
                right: right.node,
            },
            ty,
            fingerprint: std::sync::OnceLock::new(),
        },
        binding,
    )
}

fn binary_truth<T>(op: BinaryOp, left: Expr<T>, right: Expr<T>) -> Expr<Truth> {
    Expr::from_node(
        ExprNode {
            kind: ExprKind::Binary {
                op,
                left: left.node,
                right: right.node,
            },
            ty: Truth::type_def(),
            fingerprint: std::sync::OnceLock::new(),
        },
        Binding::<Truth>::native(),
    )
}
