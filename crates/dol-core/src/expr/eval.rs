use crate::diagnostic::{Diagnostic, Result};
use crate::model::RecordView;
use crate::plan::PlanId;
use crate::semantics::Truth;
use crate::types::Presence;
use crate::value::{Datum, Value, validate_datum};

use super::author::Expr;
use super::prepare::{
    BindContext, EvalContext, ExprId, PreparedExpr, PreparedExpression, PreparedKind,
};
use super::semantic::{eval_binary, eval_conditional, eval_membership, eval_unary};

impl PreparedExpression {
    pub(crate) fn evaluate_datum(&self, context: &EvalContext<'_>) -> Result<Datum> {
        self.evaluate_datum_with_exists(context, &mut |_| {
            Err(Diagnostic::error(
                "EXPR-EVAL-112",
                "existential expressions require pipeline execution context",
            ))
        })
    }

    pub(crate) fn evaluate_datum_with_exists(
        &self,
        context: &EvalContext<'_>,
        resolve_exists: &mut dyn FnMut(PlanId) -> Result<Truth>,
    ) -> Result<Datum> {
        self.validate_evaluation_scopes(context)?;
        let mut values = vec![None; self.nodes.len()];
        self.evaluate_node(self.root, context, &mut values, resolve_exists)
    }

    fn evaluate_node(
        &self,
        id: ExprId,
        context: &EvalContext<'_>,
        values: &mut [Option<Datum>],
        resolve_exists: &mut dyn FnMut(PlanId) -> Result<Truth>,
    ) -> Result<Datum> {
        if let Some(value) = values.get(id.index()).cloned().flatten() {
            return Ok(value);
        }

        let node = self.nodes.get(id.index()).ok_or_else(|| {
            Diagnostic::error(
                "EXPR-EVAL-107",
                "prepared expression references an invalid node",
            )
        })?;

        let value = match &node.kind {
            PreparedKind::Field { scope, slot, .. } => {
                let record = context.records.get(scope.index()).ok_or_else(|| {
                    Diagnostic::error(
                        "EXPR-EVAL-101",
                        "evaluation context is missing a bound record scope",
                    )
                })?;
                record
                    .field(*slot)?
                    .ok_or_else(|| {
                        Diagnostic::error(
                            "EXPR-EVAL-102",
                            "bound field slot is not exposed by the record",
                        )
                    })?
                    .into_owned_datum()
            }
            PreparedKind::Literal(value) => value.clone(),
            PreparedKind::Parameter(name) => {
                let parameter =
                    context
                        .parameters
                        .as_ref()
                        .get(name.as_ref())
                        .ok_or_else(|| {
                            Diagnostic::error(
                                "EXPR-EVAL-103",
                                format!("parameter `{name}` is not bound"),
                            )
                        })?;
                if parameter.ty != node.ty {
                    return Err(Diagnostic::error(
                        "EXPR-EVAL-111",
                        format!("parameter `{name}` was bound with a different semantic type"),
                    ));
                }
                parameter.datum.clone()
            }
            PreparedKind::Unary { op, input } => {
                let input = self.evaluate_node(*input, context, values, resolve_exists)?;
                eval_unary(*op, input, &node.ty)?
            }
            PreparedKind::Binary { op, left, right } => {
                let left = self.evaluate_node(*left, context, values, resolve_exists)?;
                if *op == super::node::BinaryOp::Coalesce
                    && !matches!(left, Datum::Missing | Datum::Null)
                {
                    left
                } else {
                    let right = self.evaluate_node(*right, context, values, resolve_exists)?;
                    eval_binary(*op, left, right)?
                }
            }
            PreparedKind::Membership {
                input,
                candidates,
                negate,
            } => {
                let input = self.evaluate_node(*input, context, values, resolve_exists)?;
                let candidates = candidates
                    .iter()
                    .map(|candidate| {
                        self.evaluate_node(*candidate, context, values, resolve_exists)
                    })
                    .collect::<Result<Vec<_>>>()?;
                eval_membership(input, candidates, *negate)?
            }
            PreparedKind::Conditional {
                condition,
                when_true,
                when_false,
            } => {
                let condition = self.evaluate_node(*condition, context, values, resolve_exists)?;
                match condition {
                    Datum::Value(Value::Truth(Truth::True)) => {
                        self.evaluate_node(*when_true, context, values, resolve_exists)?
                    }
                    Datum::Value(Value::Truth(Truth::False))
                    | Datum::Value(Value::Truth(Truth::Unknown)) => {
                        self.evaluate_node(*when_false, context, values, resolve_exists)?
                    }
                    other => eval_conditional(other, Datum::Missing, Datum::Missing)?,
                }
            }
            PreparedKind::FunctionCall {
                function,
                arguments,
            } => {
                let arguments = arguments
                    .iter()
                    .map(|argument| self.evaluate_node(*argument, context, values, resolve_exists))
                    .collect::<Result<Vec<_>>>()?;
                function.evaluate(arguments, &node.ty)?
            }
            PreparedKind::Exists { subquery, .. } => {
                Datum::Value(Value::Truth(resolve_exists(*subquery)?))
            }
        };

        let null_extended_field = match &node.kind {
            PreparedKind::Field {
                scope,
                allow_null_extension: true,
                ..
            } if matches!(value, Datum::Null) => context
                .null_extended
                .get(scope.index())
                .copied()
                .unwrap_or(false),
            _ => false,
        };
        if !null_extended_field {
            validate_datum(&node.ty, Presence::Optional, &value).map_err(|error| {
                Diagnostic::error(
                    "EXPR-EVAL-110",
                    format!(
                        "expression node produced an invalid semantic value: {}",
                        error.message()
                    ),
                )
            })?;
        }
        let slot = values.get_mut(id.index()).ok_or_else(|| {
            Diagnostic::error(
                "EXPR-EVAL-107",
                "prepared expression references an invalid node",
            )
        })?;
        *slot = Some(value.clone());
        Ok(value)
    }

    fn validate_evaluation_scopes(&self, context: &EvalContext<'_>) -> Result<()> {
        if context.records.len() != self.scopes.len()
            || context.null_extended.len() != context.records.len()
        {
            return Err(Diagnostic::error(
                "EXPR-EVAL-109",
                "evaluation context does not contain every bound model scope",
            ));
        }

        for (index, expected) in self.scopes.iter().enumerate() {
            let record = context.records.get(index).ok_or_else(|| {
                Diagnostic::error(
                    "EXPR-EVAL-109",
                    "evaluation context does not contain every bound model scope",
                )
            })?;
            let actual = record.model()?;
            if actual.key() != expected {
                return Err(Diagnostic::error(
                    "EXPR-EVAL-109",
                    format!(
                        "evaluation scope {index} expects model `{}` but received `{}`",
                        expected.as_str(),
                        actual.key().as_str()
                    ),
                ));
            }
        }

        Ok(())
    }
}

impl<T> PreparedExpr<T> {
    /// Evaluates this prepared expression against bound records and parameters.
    pub fn evaluate_datum(&self, context: &EvalContext<'_>) -> Result<Datum> {
        self.inner.evaluate_datum(context)
    }
}

impl PreparedExpr<Truth> {
    /// Evaluates a truth expression and returns its three-valued result.
    pub fn evaluate_truth(&self, context: &EvalContext<'_>) -> Result<Truth> {
        match self.evaluate_datum(context)? {
            Datum::Value(Value::Truth(value)) => Ok(value),
            _ => Err(Diagnostic::error(
                "EXPR-EVAL-108",
                "truth expression did not produce a truth value",
            )),
        }
    }
}

impl<T> Expr<T> {
    /// Prepares and evaluates this expression against one record.
    pub fn evaluate_datum(&self, record: &dyn RecordView) -> Result<Datum> {
        let model = record.model()?;
        let prepared = self.prepare(&BindContext::single(model))?;
        prepared.evaluate_datum(&EvalContext::single(record))
    }
}

impl Expr<Truth> {
    /// Prepares and evaluates a truth expression against one record.
    pub fn evaluate_truth(&self, record: &dyn RecordView) -> Result<Truth> {
        let model = record.model()?;
        let prepared = self.prepare(&BindContext::single(model))?;
        prepared.evaluate_truth(&EvalContext::single(record))
    }
}
