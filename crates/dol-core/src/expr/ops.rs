use crate::binding::Binding;
use crate::model::Field;
use crate::runtime::RuntimeField;
use crate::semantics::Truth;
use crate::types::DataType;

use super::author::{Expr, IntoExpr, ScopedField};
use super::node::{BinaryOp, ExprKind, ExprNode, UnaryOp};

/// Marker for semantic types whose same-type arithmetic is defined by DOL.
pub trait ArithmeticType: DataType {}

macro_rules! arithmetic_type {
    ($($ty:ty),+ $(,)?) => {
        $(impl ArithmeticType for $ty {})+
    };
}

arithmetic_type!(
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

impl<T: ArithmeticType> ArithmeticType for Option<T> {}

impl<T> Expr<T> {
    /// Compares this expression for semantic equality.
    #[must_use]
    pub fn eq(self, other: impl IntoExpr<T>) -> Expr<Truth> {
        binary_truth(BinaryOp::Eq, self, other.into_expr())
    }

    /// Compares this expression for semantic inequality.
    #[must_use]
    pub fn ne(self, other: impl IntoExpr<T>) -> Expr<Truth> {
        binary_truth(BinaryOp::Ne, self, other.into_expr())
    }

    /// Compares this expression using DOL less-than semantics.
    #[must_use]
    pub fn lt(self, other: impl IntoExpr<T>) -> Expr<Truth> {
        binary_truth(BinaryOp::Lt, self, other.into_expr())
    }

    /// Compares this expression using DOL less-than-or-equal semantics.
    #[must_use]
    pub fn le(self, other: impl IntoExpr<T>) -> Expr<Truth> {
        binary_truth(BinaryOp::Le, self, other.into_expr())
    }

    /// Compares this expression using DOL greater-than semantics.
    #[must_use]
    pub fn gt(self, other: impl IntoExpr<T>) -> Expr<Truth> {
        binary_truth(BinaryOp::Gt, self, other.into_expr())
    }

    /// Compares this expression using DOL greater-than-or-equal semantics.
    #[must_use]
    pub fn ge(self, other: impl IntoExpr<T>) -> Expr<Truth> {
        binary_truth(BinaryOp::Ge, self, other.into_expr())
    }

    /// Tests whether this expression is explicitly null.
    #[must_use]
    pub fn is_null(&self) -> Expr<Truth> {
        unary_truth(UnaryOp::IsNull, self.clone())
    }

    /// Tests whether this expression is missing from its record.
    #[must_use]
    pub fn is_missing(&self) -> Expr<Truth> {
        unary_truth(UnaryOp::IsMissing, self.clone())
    }

    /// Tests whether this expression is present, including explicit null.
    #[must_use]
    pub fn is_present(&self) -> Expr<Truth> {
        unary_truth(UnaryOp::IsPresent, self.clone())
    }
}

impl Expr<Truth> {
    /// Combines truth expressions using DOL three-valued AND.
    #[must_use]
    pub fn and(self, other: impl IntoExpr<Truth>) -> Self {
        binary_same(BinaryOp::And, self, other.into_expr())
    }

    /// Combines truth expressions using DOL three-valued OR.
    #[must_use]
    pub fn or(self, other: impl IntoExpr<Truth>) -> Self {
        binary_same(BinaryOp::Or, self, other.into_expr())
    }

    /// Negates this truth expression without changing expression representation.
    #[must_use]
    pub fn negate(self) -> Self {
        Self::from_node(
            ExprNode {
                kind: ExprKind::Unary {
                    op: UnaryOp::Not,
                    input: self.node,
                },
                ty: Truth::type_def(),
                fingerprint: std::sync::OnceLock::new(),
            },
            Binding::<Truth>::native(),
        )
    }
}

impl<T> Field<T> {
    /// Converts this field descriptor into a typed expression.
    #[must_use]
    pub fn expr(self) -> Expr<T> {
        self.into()
    }

    /// Equality comparison.
    #[must_use]
    pub fn eq(self, other: impl IntoExpr<T>) -> Expr<Truth> {
        self.expr().eq(other)
    }

    /// Inequality comparison.
    #[must_use]
    pub fn ne(self, other: impl IntoExpr<T>) -> Expr<Truth> {
        self.expr().ne(other)
    }

    /// Less-than comparison.
    #[must_use]
    pub fn lt(self, other: impl IntoExpr<T>) -> Expr<Truth> {
        self.expr().lt(other)
    }

    /// Less-than-or-equal comparison.
    #[must_use]
    pub fn le(self, other: impl IntoExpr<T>) -> Expr<Truth> {
        self.expr().le(other)
    }

    /// Greater-than comparison.
    #[must_use]
    pub fn gt(self, other: impl IntoExpr<T>) -> Expr<Truth> {
        self.expr().gt(other)
    }

    /// Greater-than-or-equal comparison.
    #[must_use]
    pub fn ge(self, other: impl IntoExpr<T>) -> Expr<Truth> {
        self.expr().ge(other)
    }

    /// Explicit null test.
    #[must_use]
    pub fn is_null(&self) -> Expr<Truth> {
        (*self).expr().is_null()
    }

    /// Missing-value test.
    #[must_use]
    pub fn is_missing(&self) -> Expr<Truth> {
        (*self).expr().is_missing()
    }

    /// Presence test.
    #[must_use]
    pub fn is_present(&self) -> Expr<Truth> {
        (*self).expr().is_present()
    }
}

impl<T> ScopedField<T> {
    /// Converts this explicitly-scoped field descriptor into a typed expression.
    #[must_use]
    pub fn expr(&self) -> Expr<T> {
        (*self).clone().into()
    }

    /// Equality comparison.
    #[must_use]
    pub fn eq(&self, other: impl IntoExpr<T>) -> Expr<Truth> {
        self.expr().eq(other)
    }

    /// Inequality comparison.
    #[must_use]
    pub fn ne(&self, other: impl IntoExpr<T>) -> Expr<Truth> {
        self.expr().ne(other)
    }

    /// Less-than comparison.
    #[must_use]
    pub fn lt(&self, other: impl IntoExpr<T>) -> Expr<Truth> {
        self.expr().lt(other)
    }

    /// Less-than-or-equal comparison.
    #[must_use]
    pub fn le(&self, other: impl IntoExpr<T>) -> Expr<Truth> {
        self.expr().le(other)
    }

    /// Greater-than comparison.
    #[must_use]
    pub fn gt(&self, other: impl IntoExpr<T>) -> Expr<Truth> {
        self.expr().gt(other)
    }

    /// Greater-than-or-equal comparison.
    #[must_use]
    pub fn ge(&self, other: impl IntoExpr<T>) -> Expr<Truth> {
        self.expr().ge(other)
    }

    /// Explicit null test.
    #[must_use]
    pub fn is_null(&self) -> Expr<Truth> {
        self.expr().is_null()
    }

    /// Missing-value test.
    #[must_use]
    pub fn is_missing(&self) -> Expr<Truth> {
        self.expr().is_missing()
    }

    /// Presence test.
    #[must_use]
    pub fn is_present(&self) -> Expr<Truth> {
        self.expr().is_present()
    }
}

impl<T> RuntimeField<T> {
    /// Converts this runtime-resolved field into a typed expression.
    #[must_use]
    pub fn expr(&self) -> Expr<T> {
        self.into()
    }

    /// Equality comparison.
    #[must_use]
    pub fn eq(&self, other: impl IntoExpr<T>) -> Expr<Truth> {
        self.expr().eq(other)
    }

    /// Inequality comparison.
    #[must_use]
    pub fn ne(&self, other: impl IntoExpr<T>) -> Expr<Truth> {
        self.expr().ne(other)
    }

    /// Less-than comparison.
    #[must_use]
    pub fn lt(&self, other: impl IntoExpr<T>) -> Expr<Truth> {
        self.expr().lt(other)
    }

    /// Less-than-or-equal comparison.
    #[must_use]
    pub fn le(&self, other: impl IntoExpr<T>) -> Expr<Truth> {
        self.expr().le(other)
    }

    /// Greater-than comparison.
    #[must_use]
    pub fn gt(&self, other: impl IntoExpr<T>) -> Expr<Truth> {
        self.expr().gt(other)
    }

    /// Greater-than-or-equal comparison.
    #[must_use]
    pub fn ge(&self, other: impl IntoExpr<T>) -> Expr<Truth> {
        self.expr().ge(other)
    }

    /// Explicit null test.
    #[must_use]
    pub fn is_null(&self) -> Expr<Truth> {
        self.expr().is_null()
    }

    /// Missing-value test.
    #[must_use]
    pub fn is_missing(&self) -> Expr<Truth> {
        self.expr().is_missing()
    }

    /// Presence test.
    #[must_use]
    pub fn is_present(&self) -> Expr<Truth> {
        self.expr().is_present()
    }
}

impl core::ops::Not for Expr<Truth> {
    type Output = Self;

    fn not(self) -> Self::Output {
        self.negate()
    }
}

macro_rules! arithmetic_operator {
    ($trait:ident, $method:ident, $op:expr) => {
        impl<T, R> core::ops::$trait<R> for Expr<T>
        where
            T: ArithmeticType,
            R: IntoExpr<T>,
        {
            type Output = Expr<T>;

            fn $method(self, rhs: R) -> Self::Output {
                binary_same($op, self, rhs.into_expr())
            }
        }

        impl<T, R> core::ops::$trait<R> for Field<T>
        where
            T: ArithmeticType,
            R: IntoExpr<T>,
        {
            type Output = Expr<T>;

            fn $method(self, rhs: R) -> Self::Output {
                binary_same($op, self.expr(), rhs.into_expr())
            }
        }

        impl<T, R> core::ops::$trait<R> for RuntimeField<T>
        where
            T: ArithmeticType,
            R: IntoExpr<T>,
        {
            type Output = Expr<T>;

            fn $method(self, rhs: R) -> Self::Output {
                binary_same($op, self.expr(), rhs.into_expr())
            }
        }

        impl<T, R> core::ops::$trait<R> for &RuntimeField<T>
        where
            T: ArithmeticType,
            R: IntoExpr<T>,
        {
            type Output = Expr<T>;

            fn $method(self, rhs: R) -> Self::Output {
                binary_same($op, self.expr(), rhs.into_expr())
            }
        }
    };
}

arithmetic_operator!(Add, add, BinaryOp::Add);
arithmetic_operator!(Sub, sub, BinaryOp::Sub);
arithmetic_operator!(Mul, mul, BinaryOp::Mul);
arithmetic_operator!(Div, div, BinaryOp::Div);

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

fn unary_truth<T>(op: UnaryOp, input: Expr<T>) -> Expr<Truth> {
    Expr::from_node(
        ExprNode {
            kind: ExprKind::Unary {
                op,
                input: input.node,
            },
            ty: Truth::type_def(),
            fingerprint: std::sync::OnceLock::new(),
        },
        Binding::<Truth>::native(),
    )
}
