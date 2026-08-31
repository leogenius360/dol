use std::sync::Arc;

use crate::diagnostic::Result;
use crate::semantics::Truth;
use crate::value::{Datum, Value};

use super::fingerprint::fingerprint_node;
use super::function::FunctionDeterminism;
use super::node::{ExprKind, ExprNode, UnaryOp};
use super::semantic::{eval_binary, eval_membership, eval_unary};

pub(crate) fn normalize_node(node: Arc<ExprNode>) -> Result<Arc<ExprNode>> {
    match &node.kind {
        ExprKind::Field(_) | ExprKind::Literal(_) | ExprKind::Parameter(_) => Ok(node),
        ExprKind::Unary { op, input } => {
            let input = normalize_node(Arc::clone(input))?;
            if *op == UnaryOp::Not
                && let ExprKind::Unary {
                    op: UnaryOp::Not,
                    input: nested,
                } = &input.kind
            {
                return Ok(Arc::clone(nested));
            }
            if let ExprKind::Literal(value) = &input.kind {
                let folded = eval_unary(*op, value.clone(), &node.ty)?;
                return Ok(Arc::new(ExprNode {
                    kind: ExprKind::Literal(folded),
                    ty: node.ty.clone(),
                }));
            }
            Ok(Arc::new(ExprNode {
                kind: ExprKind::Unary { op: *op, input },
                ty: node.ty.clone(),
            }))
        }
        ExprKind::Binary { op, left, right } => {
            let left = normalize_node(Arc::clone(left))?;
            if *op == super::node::BinaryOp::Coalesce
                && let ExprKind::Literal(left_value) = &left.kind
            {
                match left_value {
                    Datum::Value(_) => {
                        return Ok(Arc::new(ExprNode {
                            kind: ExprKind::Literal(left_value.clone()),
                            ty: node.ty.clone(),
                        }));
                    }
                    Datum::Missing | Datum::Null => return normalize_node(Arc::clone(right)),
                }
            }

            let right = normalize_node(Arc::clone(right))?;
            if let (ExprKind::Literal(left_value), ExprKind::Literal(right_value)) =
                (&left.kind, &right.kind)
            {
                let folded = eval_binary(*op, left_value.clone(), right_value.clone())?;
                return Ok(Arc::new(ExprNode {
                    kind: ExprKind::Literal(folded),
                    ty: node.ty.clone(),
                }));
            }
            Ok(Arc::new(ExprNode {
                kind: ExprKind::Binary {
                    op: *op,
                    left,
                    right,
                },
                ty: node.ty.clone(),
            }))
        }
        ExprKind::Membership {
            input,
            candidates,
            negate,
        } => {
            let input = normalize_node(Arc::clone(input))?;
            let candidates = candidates
                .iter()
                .map(|candidate| normalize_node(Arc::clone(candidate)))
                .collect::<Result<Vec<_>>>()?;
            let mut candidates = candidates
                .into_iter()
                .map(|candidate| {
                    fingerprint_node(&candidate).map(|fingerprint| (fingerprint, candidate))
                })
                .collect::<Result<Vec<_>>>()?;
            candidates.sort_by_key(|(fingerprint, _)| *fingerprint);
            let candidates = candidates
                .into_iter()
                .map(|(_, candidate)| candidate)
                .collect::<Vec<_>>();
            if let ExprKind::Literal(input_value) = &input.kind
                && candidates
                    .iter()
                    .all(|candidate| matches!(&candidate.kind, ExprKind::Literal(_)))
            {
                let values = candidates
                    .iter()
                    .filter_map(|candidate| match &candidate.kind {
                        ExprKind::Literal(value) => Some(value.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                let folded = eval_membership(input_value.clone(), values, *negate)?;
                return Ok(Arc::new(ExprNode {
                    kind: ExprKind::Literal(folded),
                    ty: node.ty.clone(),
                }));
            }
            Ok(Arc::new(ExprNode {
                kind: ExprKind::Membership {
                    input,
                    candidates,
                    negate: *negate,
                },
                ty: node.ty.clone(),
            }))
        }
        ExprKind::Conditional {
            condition,
            when_true,
            when_false,
        } => {
            let condition = normalize_node(Arc::clone(condition))?;
            if let ExprKind::Literal(Datum::Value(Value::Truth(value))) = &condition.kind {
                return match value {
                    Truth::True => normalize_node(Arc::clone(when_true)),
                    Truth::False | Truth::Unknown => normalize_node(Arc::clone(when_false)),
                };
            }

            let when_true = normalize_node(Arc::clone(when_true))?;
            let when_false = normalize_node(Arc::clone(when_false))?;
            Ok(Arc::new(ExprNode {
                kind: ExprKind::Conditional {
                    condition,
                    when_true,
                    when_false,
                },
                ty: node.ty.clone(),
            }))
        }
        ExprKind::FunctionCall {
            function,
            arguments,
        } => {
            let arguments = arguments
                .iter()
                .map(|argument| normalize_node(Arc::clone(argument)))
                .collect::<Result<Vec<_>>>()?;
            if function.definition().determinism() == FunctionDeterminism::Deterministic
                && arguments
                    .iter()
                    .all(|argument| matches!(&argument.kind, ExprKind::Literal(_)))
            {
                let values = arguments
                    .iter()
                    .filter_map(|argument| match &argument.kind {
                        ExprKind::Literal(value) => Some(value.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                let folded = function.evaluate(values, &node.ty)?;
                return Ok(Arc::new(ExprNode {
                    kind: ExprKind::Literal(folded),
                    ty: node.ty.clone(),
                }));
            }
            Ok(Arc::new(ExprNode {
                kind: ExprKind::FunctionCall {
                    function: function.clone(),
                    arguments,
                },
                ty: node.ty.clone(),
            }))
        }
        ExprKind::Exists(exists) => Ok(Arc::new(ExprNode {
            kind: ExprKind::Exists(super::node::ExistentialRef {
                subquery: Arc::clone(&exists.subquery),
            }),
            ty: node.ty.clone(),
        })),
    }
}
