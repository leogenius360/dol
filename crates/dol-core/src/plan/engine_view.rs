//! Read-only adapter view over the prepared expression IR.

use super::{LogicalExpr, PlanId};
use crate::expr::{BinaryOp, PreparedKind, UnaryOp};
use crate::types::TypeDef;

/// Engine-author read-only unary expression operation.
///
/// This is a hidden adapter SPI view over DOL's prepared expression IR. It is
/// intentionally not part of the application-facing expression vocabulary.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum LogicalUnaryOp {
    Not,
    IsNull,
    IsMissing,
    IsPresent,
    NullableLift,
    LosslessCast,
    Negate,
    Abs,
}

/// Engine-author read-only binary expression operation.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum LogicalBinaryOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    IsDistinctFrom,
    IsNotDistinctFrom,
    Coalesce,
}

/// Engine-author read-only view of one prepared expression node.
#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct LogicalExprNodeView<'a> {
    kind: LogicalExprNodeKind<'a>,
    ty: &'a TypeDef,
}

impl<'a> LogicalExprNodeView<'a> {
    #[must_use]
    pub const fn kind(&self) -> &LogicalExprNodeKind<'a> {
        &self.kind
    }

    #[must_use]
    pub const fn type_def(&self) -> &'a TypeDef {
        self.ty
    }
}

/// Engine-author read-only expression node payload.
#[doc(hidden)]
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum LogicalExprNodeKind<'a> {
    Field {
        scope: usize,
        slot: usize,
        allow_null_extension: bool,
    },
    Literal(&'a crate::value::Datum),
    Parameter(&'a str),
    Unary {
        op: LogicalUnaryOp,
        input: usize,
    },
    Binary {
        op: LogicalBinaryOp,
        left: usize,
        right: usize,
    },
    Membership {
        input: usize,
        candidates: Vec<usize>,
        negate: bool,
    },
    Conditional {
        condition: usize,
        when_true: usize,
        when_false: usize,
    },
    FunctionCall {
        function: &'a crate::expr::FunctionDef,
        arguments: Vec<usize>,
    },
    Exists {
        subquery: PlanId,
    },
}

/// Engine-author read-only view over a prepared logical expression.
#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct LogicalExprView<'a> {
    nodes: Vec<LogicalExprNodeView<'a>>,
    root: usize,
    scopes: &'a [crate::model::ModelKey],
    outer_scope_count: usize,
}

impl<'a> LogicalExprView<'a> {
    #[must_use]
    pub fn nodes(&self) -> &[LogicalExprNodeView<'a>] {
        &self.nodes
    }

    #[must_use]
    pub const fn root(&self) -> usize {
        self.root
    }

    #[must_use]
    pub const fn scopes(&self) -> &'a [crate::model::ModelKey] {
        self.scopes
    }

    #[must_use]
    pub const fn outer_scope_count(&self) -> usize {
        self.outer_scope_count
    }
}

impl LogicalExpr {
    /// Returns a read-only, adapter-oriented view of this prepared expression.
    ///
    /// Backends use this to compile DOL semantics without depending on private
    /// authoring nodes or reconstructing expressions from display strings.
    #[doc(hidden)]
    #[must_use]
    pub fn engine_view(&self) -> LogicalExprView<'_> {
        let nodes = self
            .prepared
            .nodes
            .iter()
            .map(|node| LogicalExprNodeView {
                kind: logical_expr_node_kind(&node.kind),
                ty: &node.ty,
            })
            .collect();
        LogicalExprView {
            nodes,
            root: self.prepared.root.index(),
            scopes: &self.prepared.scopes,
            outer_scope_count: self.prepared.outer_scope_count,
        }
    }
}

fn logical_expr_node_kind(kind: &PreparedKind) -> LogicalExprNodeKind<'_> {
    match kind {
        PreparedKind::Field {
            scope,
            slot,
            allow_null_extension,
        } => LogicalExprNodeKind::Field {
            scope: scope.index(),
            slot: slot.index(),
            allow_null_extension: *allow_null_extension,
        },
        PreparedKind::Literal(value) => LogicalExprNodeKind::Literal(value),
        PreparedKind::Parameter(name) => LogicalExprNodeKind::Parameter(name.as_ref()),
        PreparedKind::Unary { op, input } => LogicalExprNodeKind::Unary {
            op: match op {
                UnaryOp::Not => LogicalUnaryOp::Not,
                UnaryOp::IsNull => LogicalUnaryOp::IsNull,
                UnaryOp::IsMissing => LogicalUnaryOp::IsMissing,
                UnaryOp::IsPresent => LogicalUnaryOp::IsPresent,
                UnaryOp::NullableLift => LogicalUnaryOp::NullableLift,
                UnaryOp::LosslessCast => LogicalUnaryOp::LosslessCast,
                UnaryOp::Negate => LogicalUnaryOp::Negate,
                UnaryOp::Abs => LogicalUnaryOp::Abs,
            },
            input: input.index(),
        },
        PreparedKind::Binary { op, left, right } => LogicalExprNodeKind::Binary {
            op: match op {
                BinaryOp::Eq => LogicalBinaryOp::Eq,
                BinaryOp::Ne => LogicalBinaryOp::Ne,
                BinaryOp::Lt => LogicalBinaryOp::Lt,
                BinaryOp::Le => LogicalBinaryOp::Le,
                BinaryOp::Gt => LogicalBinaryOp::Gt,
                BinaryOp::Ge => LogicalBinaryOp::Ge,
                BinaryOp::And => LogicalBinaryOp::And,
                BinaryOp::Or => LogicalBinaryOp::Or,
                BinaryOp::Add => LogicalBinaryOp::Add,
                BinaryOp::Sub => LogicalBinaryOp::Sub,
                BinaryOp::Mul => LogicalBinaryOp::Mul,
                BinaryOp::Div => LogicalBinaryOp::Div,
                BinaryOp::Rem => LogicalBinaryOp::Rem,
                BinaryOp::IsDistinctFrom => LogicalBinaryOp::IsDistinctFrom,
                BinaryOp::IsNotDistinctFrom => LogicalBinaryOp::IsNotDistinctFrom,
                BinaryOp::Coalesce => LogicalBinaryOp::Coalesce,
            },
            left: left.index(),
            right: right.index(),
        },
        PreparedKind::Membership {
            input,
            candidates,
            negate,
        } => LogicalExprNodeKind::Membership {
            input: input.index(),
            candidates: candidates.iter().map(|id| id.index()).collect(),
            negate: *negate,
        },
        PreparedKind::Conditional {
            condition,
            when_true,
            when_false,
        } => LogicalExprNodeKind::Conditional {
            condition: condition.index(),
            when_true: when_true.index(),
            when_false: when_false.index(),
        },
        PreparedKind::FunctionCall {
            function,
            arguments,
        } => LogicalExprNodeKind::FunctionCall {
            function: function.definition(),
            arguments: arguments.iter().map(|id| id.index()).collect(),
        },
        PreparedKind::Exists { subquery, .. } => LogicalExprNodeKind::Exists {
            subquery: *subquery,
        },
    }
}
