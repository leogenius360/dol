//! Existential expression authoring and pipeline-dependency traversal.

use std::sync::Arc;

use crate::binding::Binding;
use crate::pipeline::{PipelineNode, fingerprint_authoring_node};
use crate::semantics::Truth;
use crate::types::DataType;

use super::author::{Expr, ExpressionSpec};
use super::node::{BinaryOp, ExistentialRef, ExprKind, ExprNode};

impl ExpressionSpec {
    pub(crate) fn canonical_truth(self) -> Self {
        let fallback = Arc::clone(&self.node);
        let mut leaves = Vec::new();
        let mut stack = vec![self.node];
        while let Some(node) = stack.pop() {
            match &node.kind {
                ExprKind::Binary {
                    op: BinaryOp::And,
                    left,
                    right,
                } => {
                    stack.push(Arc::clone(right));
                    stack.push(Arc::clone(left));
                }
                _ => leaves.push(node),
            }
        }

        while leaves.len() > 1 {
            let mut next = Vec::with_capacity(leaves.len().div_ceil(2));
            let mut pairs = leaves.into_iter();
            while let Some(left) = pairs.next() {
                match pairs.next() {
                    Some(right) => next.push(Arc::new(ExprNode {
                        kind: ExprKind::Binary {
                            op: BinaryOp::And,
                            left,
                            right,
                        },
                        ty: Truth::type_def(),
                    })),
                    None => next.push(left),
                }
            }
            leaves = next;
        }

        Self {
            node: leaves.pop().unwrap_or(fallback),
            ty: Truth::type_def(),
        }
    }

    pub(crate) fn and_truth(self, other: Self) -> Self {
        Self {
            node: Arc::new(ExprNode {
                kind: ExprKind::Binary {
                    op: BinaryOp::And,
                    left: self.node,
                    right: other.node,
                },
                ty: Truth::type_def(),
            }),
            ty: Truth::type_def(),
        }
        .canonical_truth()
    }

    pub(crate) fn push_pipeline_inputs(&self, output: &mut Vec<Arc<PipelineNode>>) {
        let mut stack = vec![self.node.as_ref()];
        while let Some(node) = stack.pop() {
            match &node.kind {
                ExprKind::Field(_) | ExprKind::Literal(_) | ExprKind::Parameter(_) => {}
                ExprKind::Unary { input, .. } => stack.push(input),
                ExprKind::Binary { left, right, .. } => {
                    stack.push(right);
                    stack.push(left);
                }
                ExprKind::Membership {
                    input, candidates, ..
                } => {
                    for candidate in candidates.iter().rev() {
                        stack.push(candidate);
                    }
                    stack.push(input);
                }
                ExprKind::Conditional {
                    condition,
                    when_true,
                    when_false,
                } => {
                    stack.push(when_false);
                    stack.push(when_true);
                    stack.push(condition);
                }
                ExprKind::FunctionCall { arguments, .. } => {
                    for argument in arguments.iter().rev() {
                        stack.push(argument);
                    }
                }
                ExprKind::Exists(exists) => {
                    output.push(Arc::clone(&exists.subquery));
                }
            }
        }
    }
}

impl Expr<Truth> {
    pub(crate) fn existential(subquery: Arc<PipelineNode>) -> Self {
        let binding = Binding::<Truth>::native();
        Self::from_node(
            ExprNode {
                kind: ExprKind::Exists(ExistentialRef { subquery }),
                ty: binding.type_def(),
            },
            binding,
        )
    }
}

impl ExistentialRef {
    pub(crate) fn subquery(&self) -> &Arc<PipelineNode> {
        &self.subquery
    }

    pub(crate) fn subquery_fingerprint(
        &self,
    ) -> crate::diagnostic::Result<crate::fingerprint::Fingerprint> {
        fingerprint_authoring_node(&self.subquery)
    }
}
