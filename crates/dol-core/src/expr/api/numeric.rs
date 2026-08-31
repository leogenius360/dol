use crate::binding::Binding;
use crate::value::DataValue;

use super::super::advanced::{LosslessNumericCastType, NegatableType};
use super::super::author::{Expr, ExprSource};
use super::super::node::{ExprKind, ExprNode, UnaryOp};

/// Type-owned symbolic numeric methods available to all losslessly-castable numeric sources.
pub trait NumericExprExt: ExprSource + Sized
where
    Self::Value: LosslessNumericCastType,
{
    /// Explicitly casts to another numeric semantic type when every source value is representable.
    #[must_use]
    fn cast<U>(self) -> Expr<U>
    where
        U: LosslessNumericCastType + DataValue,
    {
        let input = self.into_expression();
        let binding = Binding::<U>::native();
        Expr::from_node(
            ExprNode {
                kind: ExprKind::Unary {
                    op: UnaryOp::LosslessCast,
                    input: input.node,
                },
                ty: binding.type_def(),
            },
            binding,
        )
    }
}

impl<S> NumericExprExt for S
where
    S: ExprSource + Sized,
    S::Value: LosslessNumericCastType,
{
}

/// Symbolic methods shared by signed numeric domains.
pub trait SignedNumericExprExt: ExprSource + Sized
where
    Self::Value: NegatableType,
{
    /// Returns the absolute value using checked DOL numeric semantics.
    #[must_use]
    fn abs(self) -> Expr<Self::Value> {
        let input = self.into_expression();
        let binding = input.binding;
        Expr::from_node(
            ExprNode {
                kind: ExprKind::Unary {
                    op: UnaryOp::Abs,
                    input: input.node,
                },
                ty: binding.type_def(),
            },
            binding,
        )
    }
}

impl<S> SignedNumericExprExt for S
where
    S: ExprSource + Sized,
    S::Value: NegatableType,
{
}
