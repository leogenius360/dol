use crate::binding::{Binding, SemanticBinding};
use crate::value::DataValue;

use super::author::{Expr, ExprSource, IntoExpr};
use super::function::{BuiltinFunction, FunctionRef, SemanticFunction};
use super::node::{ExprKind, ExprNode};
use super::semantic::eval_builtin_function;
use super::validate::validate_builtin_function;

pub(crate) fn builtin_call1<S, O>(function: BuiltinFunction, source: S) -> Expr<O>
where
    S: ExprSource,
    O: DataValue,
{
    call1_with_binding(
        FunctionRef::new(
            function.definition(),
            validate_builtin_function,
            eval_builtin_function,
        ),
        source.into_expression(),
        Binding::<O>::native(),
    )
}

pub(crate) fn builtin_call2<S, R, O>(
    function: BuiltinFunction,
    source: S,
    right: impl IntoExpr<R>,
) -> Expr<O>
where
    S: ExprSource,
    O: DataValue,
{
    call2_with_binding(
        FunctionRef::new(
            function.definition(),
            validate_builtin_function,
            eval_builtin_function,
        ),
        source.into_expression(),
        right.into_expr(),
        Binding::<O>::native(),
    )
}

/// Calls a unary primitive semantic function from a symbolic source.
///
/// Type-owned expression APIs normally wrap this helper so application code keeps
/// calling domain methods such as `Invoice::total.currency()` rather than generic
/// function plumbing.
#[must_use]
pub fn call1<F, O>(source: impl ExprSource) -> Expr<O>
where
    F: SemanticFunction,
    O: DataValue,
{
    call1_with_binding(
        FunctionRef::semantic::<F>(),
        source.into_expression(),
        Binding::<O>::native(),
    )
}

/// Calls a unary primitive semantic function with an explicit result binding.
#[must_use]
pub fn call1_with<F, O, B>(source: impl ExprSource) -> Expr<O>
where
    F: SemanticFunction,
    B: SemanticBinding<O>,
{
    call1_with_binding(
        FunctionRef::semantic::<F>(),
        source.into_expression(),
        Binding::<O>::of::<B>(),
    )
}

/// Calls a binary primitive semantic function from a symbolic source and right operand.
#[must_use]
pub fn call2<F, R, O>(source: impl ExprSource, right: impl IntoExpr<R>) -> Expr<O>
where
    F: SemanticFunction,
    O: DataValue,
{
    call2_with_binding(
        FunctionRef::semantic::<F>(),
        source.into_expression(),
        right.into_expr(),
        Binding::<O>::native(),
    )
}

/// Calls a binary primitive semantic function with an explicit result binding.
#[must_use]
pub fn call2_with<F, R, O, B>(source: impl ExprSource, right: impl IntoExpr<R>) -> Expr<O>
where
    F: SemanticFunction,
    B: SemanticBinding<O>,
{
    call2_with_binding(
        FunctionRef::semantic::<F>(),
        source.into_expression(),
        right.into_expr(),
        Binding::<O>::of::<B>(),
    )
}

fn call1_with_binding<I, O>(function: FunctionRef, input: Expr<I>, binding: Binding<O>) -> Expr<O> {
    Expr::from_node(
        ExprNode {
            kind: ExprKind::FunctionCall {
                function,
                arguments: vec![input.node],
            },
            ty: binding.type_def(),
        },
        binding,
    )
}

fn call2_with_binding<L, R, O>(
    function: FunctionRef,
    left: Expr<L>,
    right: Expr<R>,
    binding: Binding<O>,
) -> Expr<O> {
    Expr::from_node(
        ExprNode {
            kind: ExprKind::FunctionCall {
                function,
                arguments: vec![left.node, right.node],
            },
            ty: binding.type_def(),
        },
        binding,
    )
}
