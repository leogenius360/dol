use std::sync::{Arc, OnceLock};

use crate::model::{FieldKey, ModelKey};
use crate::pipeline::PipelineNode;
use crate::types::TypeDef;
use crate::value::Datum;

use super::function::FunctionRef;

#[derive(Debug, Clone)]
pub(crate) struct ExprNode {
    pub(crate) kind: ExprKind,
    pub(crate) ty: TypeDef,
    pub(super) fingerprint: OnceLock<crate::fingerprint::Fingerprint>,
}

impl ExprNode {
    pub(crate) fn cached_fingerprint(&self) -> Option<crate::fingerprint::Fingerprint> {
        self.fingerprint.get().copied()
    }

    pub(crate) fn cache_fingerprint(
        &self,
        fingerprint: crate::fingerprint::Fingerprint,
    ) -> crate::fingerprint::Fingerprint {
        *self.fingerprint.get_or_init(|| fingerprint)
    }
}

#[derive(Debug, Clone)]
pub(crate) enum ExprKind {
    Field(FieldRef),
    Literal(Datum),
    Parameter(ParameterRef),
    Unary {
        op: UnaryOp,
        input: Arc<ExprNode>,
    },
    Binary {
        op: BinaryOp,
        left: Arc<ExprNode>,
        right: Arc<ExprNode>,
    },
    Membership {
        input: Arc<ExprNode>,
        candidates: Vec<Arc<ExprNode>>,
        negate: bool,
    },
    Conditional {
        condition: Arc<ExprNode>,
        when_true: Arc<ExprNode>,
        when_false: Arc<ExprNode>,
    },
    FunctionCall {
        function: FunctionRef,
        arguments: Vec<Arc<ExprNode>>,
    },
    Exists(ExistentialRef),
}

#[derive(Clone)]
pub(crate) struct ExistentialRef {
    pub(crate) subquery: Arc<PipelineNode>,
}

impl core::fmt::Debug for ExistentialRef {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ExistentialRef").finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct FieldRef {
    pub(crate) model: ModelKey,
    pub(crate) key: FieldKey,
    pub(crate) name: Arc<str>,
    pub(crate) scope: Option<Arc<str>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ParameterRef {
    pub(crate) name: Arc<str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum UnaryOp {
    Not,
    IsNull,
    IsMissing,
    IsPresent,
    NullableLift,
    LosslessCast,
    Negate,
    Abs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum BinaryOp {
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
