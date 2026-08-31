//! Typed symbolic expression language, preparation, fingerprints, and local evaluation.

mod advanced;
pub mod api;
mod author;
mod call;
mod eval;
mod existential;
mod fingerprint;
mod function;
mod node;
mod normalize;
mod ops;
mod prepare;
mod semantic;
mod validate;

pub use advanced::{IntegralType, LosslessNumericCastType, NegatableType};
pub use api::{
    DateExprExt, DurationExprExt, InstantExprExt, LocalDateTimeExprExt, NullableDateExprExt,
    NullableDurationExprExt, NullableInstantExprExt, NullableLocalDateTimeExprExt,
    NullableStringExprExt, NumericExprExt, SignedNumericExprExt, StringExprExt,
};
#[doc(hidden)]
pub use author::ExpressionSpec;
pub use author::{Expr, ExprSource, IntoExpr, IntoTypedExpr, Parameter, ScopedField};
pub use call::{call1, call1_with, call2, call2_with};
pub(crate) use fingerprint::fingerprint_node as fingerprint_expression_node;
pub use function::{FunctionDef, FunctionDeterminism, FunctionKey, SemanticFunction};
pub(crate) use node::{BinaryOp, ExistentialRef, UnaryOp};
pub use ops::ArithmeticType;
pub use prepare::{BindContext, BoundParameter, EvalContext, Parameters, PreparedExpr};
pub(crate) use prepare::{
    PreparedExistential, PreparedExpression, PreparedKind, prepare_expression,
    prepare_expression_with_existentials,
};

use crate::fingerprint::Fingerprint;

impl<T> Expr<T> {
    /// Canonical semantic fingerprint of the validated, normalized unbound expression.
    pub fn fingerprint(&self) -> crate::diagnostic::Result<Fingerprint> {
        let normalized = self.normalized()?;
        fingerprint::fingerprint_node(&normalized.node)
    }

    /// Returns a normalized expression using semantics-preserving Phase-2 rewrites.
    pub fn normalized(&self) -> crate::diagnostic::Result<Self> {
        self.normalized_with_limits(crate::limits::ExpressionLimits::default())
    }

    /// Normalizes this expression under explicit resource limits.
    pub fn normalized_with_limits(
        &self,
        limits: crate::limits::ExpressionLimits,
    ) -> crate::diagnostic::Result<Self> {
        prepare::check_expression_limits(&self.node, limits)?;
        validate::validate_expression_node(&self.node, limits)?;
        let normalized = normalize::normalize_node(std::sync::Arc::clone(&self.node))?;
        validate::validate_expression_node(&normalized, limits)?;
        if normalized.ty != self.binding.type_def() {
            return Err(crate::diagnostic::Diagnostic::error(
                "EXPR-TYPE-001",
                "expression's semantic result type disagrees with its Rust type parameter",
            ));
        }
        Ok(Self::from_arc(normalized, self.binding))
    }
}

pub(crate) fn fingerprint_expression_spec(
    expression: &ExpressionSpec,
) -> crate::diagnostic::Result<Fingerprint> {
    let limits = crate::limits::ExpressionLimits::default();
    prepare::check_expression_limits(&expression.node, limits)?;
    validate::validate_expression_node(&expression.node, limits)?;
    let normalized = normalize::normalize_node(std::sync::Arc::clone(&expression.node))?;
    validate::validate_expression_node(&normalized, limits)?;
    if normalized.ty != expression.ty {
        return Err(crate::diagnostic::Diagnostic::error(
            "EXPR-TYPE-001",
            "expression's semantic result type disagrees with its authoring type",
        ));
    }
    fingerprint_expression_node(&normalized)
}
