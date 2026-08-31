use core::marker::PhantomData;
use std::borrow::Cow;
use std::sync::Arc;

use crate::diagnostic::{Diagnostic, Result};
use crate::fingerprint::{CanonicalHasher, Fingerprint};
use crate::limits::ExpressionLimits;
use crate::model::{FieldSlot, ModelDef, ModelKey, RecordView};
use crate::plan::PlanId;
use crate::types::TypeDef;
use crate::value::Datum;

use super::author::{Expr, ExpressionSpec};
use super::fingerprint::fingerprint_node;
use super::function::FunctionRef;
use super::node::{BinaryOp, ExistentialRef, ExprKind, ExprNode, FieldRef, UnaryOp};
use super::normalize::normalize_node;
use super::validate::validate_expression_node;

/// Dense expression-local identifier used by prepared expressions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct ExprId(u32);

impl ExprId {
    fn from_index(index: usize) -> Result<Self> {
        let value = u32::try_from(index).map_err(|_| {
            Diagnostic::error(
                "EXPR-LIMIT-003",
                "prepared expression exceeds u32 node capacity",
            )
        })?;
        Ok(Self(value))
    }

    /// Zero-based prepared-node index.
    #[must_use]
    pub(crate) const fn index(self) -> usize {
        self.0 as usize
    }
}

/// Pipeline-plan-local source scope identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct ScopeId(u32);

impl ScopeId {
    fn from_index(index: usize) -> Result<Self> {
        let value = u32::try_from(index).map_err(|_| {
            Diagnostic::error("EXPR-SCOPE-004", "expression scope exceeds u32 capacity")
        })?;
        Ok(Self(value))
    }

    /// Zero-based scope index.
    #[must_use]
    pub(crate) const fn index(self) -> usize {
        self.0 as usize
    }
}

/// Models available while resolving field references in an expression.
#[derive(Debug, Clone, Default)]
pub struct BindContext<'a> {
    models: Vec<&'a ModelDef>,
    aliases: Vec<Option<Arc<str>>>,
    nullable: Vec<bool>,
}

impl<'a> BindContext<'a> {
    /// Creates an empty binding context.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            models: Vec::new(),
            aliases: Vec::new(),
            nullable: Vec::new(),
        }
    }

    /// Creates a single-model binding context.
    #[must_use]
    pub fn single(model: &'a ModelDef) -> Self {
        Self {
            models: vec![model],
            aliases: vec![None],
            nullable: vec![false],
        }
    }

    /// Adds one model source in scope order.
    #[must_use]
    pub fn with_model(mut self, model: &'a ModelDef) -> Self {
        self.models.push(model);
        self.aliases.push(None);
        self.nullable.push(false);
        self
    }

    /// Adds one nullable model source in scope order.
    #[must_use]
    pub fn with_nullable_model(mut self, model: &'a ModelDef) -> Self {
        self.models.push(model);
        self.aliases.push(None);
        self.nullable.push(true);
        self
    }

    /// Adds one explicitly-aliased model source in scope order.
    #[must_use]
    pub fn with_alias(mut self, alias: impl Into<Arc<str>>, model: &'a ModelDef) -> Self {
        self.models.push(model);
        self.aliases.push(Some(alias.into()));
        self.nullable.push(false);
        self
    }

    /// Adds one nullable explicitly-aliased model source in scope order.
    #[must_use]
    pub fn with_nullable_alias(mut self, alias: impl Into<Arc<str>>, model: &'a ModelDef) -> Self {
        self.models.push(model);
        self.aliases.push(Some(alias.into()));
        self.nullable.push(true);
        self
    }

    /// Models in scope order.
    #[must_use]
    pub fn models(&self) -> &[&'a ModelDef] {
        &self.models
    }

    fn resolve_field(&self, field: &FieldRef) -> Result<(ScopeId, &'a ModelDef, bool)> {
        let mut found = None;
        for (index, model) in self.models.iter().copied().enumerate() {
            if model.key() != &field.model {
                continue;
            }
            if let Some(required) = field.scope.as_deref()
                && self.aliases.get(index).and_then(Option::as_deref) != Some(required)
            {
                continue;
            }
            if found.is_some() {
                return Err(Diagnostic::error(
                    "EXPR-SCOPE-002",
                    format!(
                        "model `{}` appears more than once; bind the field to an explicit pipeline source alias",
                        field.model.as_str()
                    ),
                ));
            }
            found = Some((
                ScopeId::from_index(index)?,
                model,
                self.nullable.get(index).copied().unwrap_or(false),
            ));
        }

        found.ok_or_else(|| {
            if let Some(alias) = field.scope.as_deref() {
                Diagnostic::error(
                    "EXPR-SCOPE-005",
                    format!(
                        "source alias `{alias}` for model `{}` is not available in this expression scope",
                        field.model.as_str()
                    ),
                )
            } else {
                Diagnostic::error(
                    "EXPR-SCOPE-001",
                    format!(
                        "model `{}` is not available in this expression scope",
                        field.model.as_str()
                    ),
                )
            }
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PreparedNode {
    pub(crate) kind: PreparedKind,
    pub(crate) ty: TypeDef,
}

#[derive(Debug, Clone)]
pub(crate) enum PreparedKind {
    Field {
        scope: ScopeId,
        slot: FieldSlot,
        allow_null_extension: bool,
    },
    Literal(Datum),
    Parameter(Arc<str>),
    Unary {
        op: UnaryOp,
        input: ExprId,
    },
    Binary {
        op: BinaryOp,
        left: ExprId,
        right: ExprId,
    },
    Membership {
        input: ExprId,
        candidates: Box<[ExprId]>,
        negate: bool,
    },
    Conditional {
        condition: ExprId,
        when_true: ExprId,
        when_false: ExprId,
    },
    FunctionCall {
        function: FunctionRef,
        arguments: Box<[ExprId]>,
    },
    Exists {
        subquery: PlanId,
        fingerprint: Fingerprint,
    },
}

/// Type-erased validated expression retained by logical plans.
#[derive(Debug, Clone)]
pub(crate) struct PreparedExpression {
    pub(crate) nodes: Box<[PreparedNode]>,
    pub(crate) root: ExprId,
    pub(crate) ty: TypeDef,
    pub(crate) expression_fingerprint: Fingerprint,
    pub(crate) bound_fingerprint: Fingerprint,
    pub(crate) scopes: Box<[ModelKey]>,
    pub(crate) outer_scope_count: usize,
}

/// Existential dependency resolved by the pipeline compiler while preparing a filter expression.
pub(crate) struct PreparedExistential {
    pub(crate) subquery: PlanId,
    pub(crate) fingerprint: Fingerprint,
}

/// Validated, bound and flat expression ready for local or engine compilation.
pub struct PreparedExpr<T> {
    pub(crate) inner: PreparedExpression,
    _marker: PhantomData<fn() -> T>,
}

impl<T> Clone for PreparedExpr<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            _marker: PhantomData,
        }
    }
}

impl<T> core::fmt::Debug for PreparedExpr<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PreparedExpr")
            .field("inner", &self.inner)
            .finish_non_exhaustive()
    }
}

impl<T> PreparedExpr<T> {
    /// Exact semantic result type.
    #[must_use]
    pub const fn type_def(&self) -> &TypeDef {
        &self.inner.ty
    }

    /// Canonical semantic fingerprint, independent of local slots and node IDs.
    #[must_use]
    pub const fn expression_fingerprint(&self) -> Fingerprint {
        self.inner.expression_fingerprint
    }

    /// Number of flat prepared nodes.
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.inner.nodes.len()
    }

    /// Number of model scopes captured during binding.
    #[must_use]
    pub fn scope_count(&self) -> usize {
        self.inner.scopes.len()
    }
}

impl<T> Expr<T> {
    /// Validates, normalizes and binds this expression to the available models.
    pub fn prepare(&self, context: &BindContext<'_>) -> Result<PreparedExpr<T>> {
        self.prepare_with_limits(context, ExpressionLimits::default())
    }

    /// Prepares this expression under explicit resource limits.
    pub fn prepare_with_limits(
        &self,
        context: &BindContext<'_>,
        limits: ExpressionLimits,
    ) -> Result<PreparedExpr<T>> {
        Ok(PreparedExpr {
            inner: prepare_expression(&self.spec(), context, limits)?,
            _marker: PhantomData,
        })
    }

    /// Convenience preparation for a single model source.
    pub fn prepare_for(&self, model: &ModelDef) -> Result<PreparedExpr<T>> {
        self.prepare(&BindContext::single(model))
    }
}

pub(crate) fn prepare_expression(
    expression: &ExpressionSpec,
    context: &BindContext<'_>,
    limits: ExpressionLimits,
) -> Result<PreparedExpression> {
    let mut reject_existential = |_exists: &ExistentialRef| {
        Err(Diagnostic::error(
            "EXPR-EXISTS-001",
            "existential expressions can only be bound while planning a pipeline filter",
        ))
    };
    prepare_expression_with_existentials(expression, context, limits, 0, &mut reject_existential)
}

pub(crate) fn prepare_expression_with_existentials<F>(
    expression: &ExpressionSpec,
    context: &BindContext<'_>,
    limits: ExpressionLimits,
    outer_scope_count: usize,
    resolve_existential: &mut F,
) -> Result<PreparedExpression>
where
    F: FnMut(&ExistentialRef) -> Result<PreparedExistential>,
{
    if outer_scope_count > context.models().len() {
        return Err(Diagnostic::error(
            "EXPR-SCOPE-007",
            "outer expression scope count exceeds the available binding context",
        ));
    }

    let nodes = check_expression_limits(&expression.node, limits)?;

    validate_expression_node(&expression.node, limits)?;
    let normalized = normalize_node(Arc::clone(&expression.node))?;
    validate_expression_node(&normalized, limits)?;
    if normalized.ty != expression.ty {
        return Err(Diagnostic::error(
            "EXPR-TYPE-001",
            "expression's semantic result type disagrees with its Rust type parameter",
        ));
    }

    let fingerprint = fingerprint_node(&normalized)?;
    let mut prepared = Vec::with_capacity(nodes);
    let root = compile_node(&normalized, context, &mut prepared, resolve_existential)?;
    let bound_fingerprint = fingerprint_bound_expression(fingerprint, &prepared, outer_scope_count);
    Ok(PreparedExpression {
        nodes: prepared.into_boxed_slice(),
        root,
        ty: normalized.ty.clone(),
        expression_fingerprint: fingerprint,
        bound_fingerprint,
        scopes: context
            .models()
            .iter()
            .map(|model| model.key().clone())
            .collect::<Vec<_>>()
            .into_boxed_slice(),
        outer_scope_count,
    })
}

fn compile_node<F>(
    node: &Arc<ExprNode>,
    context: &BindContext<'_>,
    output: &mut Vec<PreparedNode>,
    resolve_existential: &mut F,
) -> Result<ExprId>
where
    F: FnMut(&ExistentialRef) -> Result<PreparedExistential>,
{
    compile_node_mode(node, context, output, false, resolve_existential)
}

fn compile_node_mode<F>(
    node: &Arc<ExprNode>,
    context: &BindContext<'_>,
    output: &mut Vec<PreparedNode>,
    allow_nullable_field: bool,
    resolve_existential: &mut F,
) -> Result<ExprId>
where
    F: FnMut(&ExistentialRef) -> Result<PreparedExistential>,
{
    let kind = match &node.kind {
        ExprKind::Field(field) => {
            let (scope, model, nullable_scope) = context.resolve_field(field)?;
            let definition = model.field(field.key.as_str()).ok_or_else(|| {
                Diagnostic::error(
                    "EXPR-SCOPE-003",
                    format!(
                        "field `{}` is not defined by model `{}`",
                        field.key.as_str(),
                        field.model.as_str()
                    ),
                )
            })?;
            if definition.ty() != &node.ty {
                return Err(Diagnostic::error(
                    "EXPR-TYPE-002",
                    format!(
                        "field `{}` semantic type does not match its model definition",
                        field.name
                    ),
                ));
            }
            if nullable_scope && !definition.ty().is_nullable() && !allow_nullable_field {
                return Err(Diagnostic::error(
                    "EXPR-SCOPE-006",
                    format!(
                        "field `{}` is read from the nullable side of an outer join; \
                         lift it explicitly with `.nullable()`",
                        field.name
                    ),
                ));
            }
            PreparedKind::Field {
                scope,
                slot: definition.slot(),
                allow_null_extension: nullable_scope
                    && !definition.ty().is_nullable()
                    && allow_nullable_field,
            }
        }
        ExprKind::Literal(value) => PreparedKind::Literal(value.clone()),
        ExprKind::Parameter(parameter) => PreparedKind::Parameter(Arc::clone(&parameter.name)),
        ExprKind::Unary { op, input } => PreparedKind::Unary {
            op: *op,
            input: compile_node_mode(
                input,
                context,
                output,
                matches!(op, super::node::UnaryOp::NullableLift),
                resolve_existential,
            )?,
        },
        ExprKind::Binary { op, left, right } => PreparedKind::Binary {
            op: *op,
            left: compile_node_mode(left, context, output, false, resolve_existential)?,
            right: compile_node_mode(right, context, output, false, resolve_existential)?,
        },
        ExprKind::Membership {
            input,
            candidates,
            negate,
        } => {
            let input = compile_node_mode(input, context, output, false, resolve_existential)?;
            let candidates = candidates
                .iter()
                .map(|candidate| {
                    compile_node_mode(candidate, context, output, false, resolve_existential)
                })
                .collect::<Result<Vec<_>>>()?
                .into_boxed_slice();
            PreparedKind::Membership {
                input,
                candidates,
                negate: *negate,
            }
        }
        ExprKind::Conditional {
            condition,
            when_true,
            when_false,
        } => PreparedKind::Conditional {
            condition: compile_node_mode(condition, context, output, false, resolve_existential)?,
            when_true: compile_node_mode(when_true, context, output, false, resolve_existential)?,
            when_false: compile_node_mode(when_false, context, output, false, resolve_existential)?,
        },
        ExprKind::FunctionCall {
            function,
            arguments,
        } => PreparedKind::FunctionCall {
            function: function.clone(),
            arguments: arguments
                .iter()
                .map(|argument| {
                    compile_node_mode(argument, context, output, false, resolve_existential)
                })
                .collect::<Result<Vec<_>>>()?
                .into_boxed_slice(),
        },
        ExprKind::Exists(exists) => {
            let resolved = resolve_existential(exists)?;
            PreparedKind::Exists {
                subquery: resolved.subquery,
                fingerprint: resolved.fingerprint,
            }
        }
    };

    let id = ExprId::from_index(output.len())?;
    output.push(PreparedNode {
        kind,
        ty: node.ty.clone(),
    });
    Ok(id)
}

fn fingerprint_bound_expression(
    expression: Fingerprint,
    nodes: &[PreparedNode],
    outer_scope_count: usize,
) -> Fingerprint {
    let mut hasher = CanonicalHasher::new(b"expression/binding/v3");
    hasher.bytes(expression.as_bytes());
    hasher.u64(outer_scope_count as u64);
    let field_count = nodes
        .iter()
        .filter(|node| matches!(&node.kind, PreparedKind::Field { .. }))
        .count();
    hasher.u64(field_count as u64);
    for node in nodes {
        if let PreparedKind::Field {
            scope,
            allow_null_extension,
            ..
        } = &node.kind
        {
            hasher.u64(scope.index() as u64);
            hasher.u8(u8::from(*allow_null_extension));
        }
    }
    let existential_count = nodes
        .iter()
        .filter(|node| matches!(&node.kind, PreparedKind::Exists { .. }))
        .count();
    hasher.u64(existential_count as u64);
    for node in nodes {
        if let PreparedKind::Exists { fingerprint, .. } = &node.kind {
            hasher.bytes(fingerprint.as_bytes());
        }
    }
    hasher.finish()
}

pub(crate) fn check_expression_limits(
    node: &Arc<ExprNode>,
    limits: ExpressionLimits,
) -> Result<usize> {
    let mut stack = vec![(node.as_ref(), 1_usize)];
    let mut nodes = 0_usize;

    while let Some((current, depth)) = stack.pop() {
        nodes = nodes.saturating_add(1);
        if nodes > limits.max_nodes {
            return Err(Diagnostic::error(
                "EXPR-LIMIT-001",
                format!("expression has more than {} nodes", limits.max_nodes),
            ));
        }
        if depth > limits.max_depth {
            return Err(Diagnostic::error(
                "EXPR-LIMIT-002",
                format!("expression depth exceeds limit {}", limits.max_depth),
            ));
        }

        match &current.kind {
            ExprKind::Field(_) | ExprKind::Literal(_) | ExprKind::Parameter(_) => {}
            ExprKind::Unary { input, .. } => {
                stack.push((input.as_ref(), depth.saturating_add(1)));
            }
            ExprKind::Binary { left, right, .. } => {
                let child_depth = depth.saturating_add(1);
                stack.push((right.as_ref(), child_depth));
                stack.push((left.as_ref(), child_depth));
            }
            ExprKind::Membership {
                input, candidates, ..
            } => {
                let child_depth = depth.saturating_add(1);
                for candidate in candidates.iter().rev() {
                    stack.push((candidate.as_ref(), child_depth));
                }
                stack.push((input.as_ref(), child_depth));
            }
            ExprKind::Conditional {
                condition,
                when_true,
                when_false,
            } => {
                let child_depth = depth.saturating_add(1);
                stack.push((when_false.as_ref(), child_depth));
                stack.push((when_true.as_ref(), child_depth));
                stack.push((condition.as_ref(), child_depth));
            }
            ExprKind::FunctionCall { arguments, .. } => {
                let child_depth = depth.saturating_add(1);
                for argument in arguments.iter().rev() {
                    stack.push((argument.as_ref(), child_depth));
                }
            }
            ExprKind::Exists(_) => {}
        }
    }

    Ok(nodes)
}

/// Canonical value bound to one named runtime parameter.
///
/// Engine adapters can inspect the semantic type and canonical datum without
/// gaining a way to construct untyped parameter bindings directly.
#[derive(Debug, Clone)]
pub struct BoundParameter {
    pub(crate) ty: TypeDef,
    pub(crate) datum: Datum,
}

impl BoundParameter {
    /// Exact semantic type validated when this parameter was bound.
    #[must_use]
    pub const fn type_def(&self) -> &TypeDef {
        &self.ty
    }

    /// Canonical DOL datum supplied for this parameter.
    #[must_use]
    pub const fn datum(&self) -> &Datum {
        &self.datum
    }
}

/// Canonical typed parameter values supplied separately from expression structure.
#[derive(Debug, Clone, Default)]
pub struct Parameters {
    values: std::collections::BTreeMap<String, BoundParameter>,
}

impl Parameters {
    /// Creates an empty parameter set.
    #[must_use]
    pub fn new() -> Self {
        Self {
            values: std::collections::BTreeMap::new(),
        }
    }

    /// Adds one typed parameter value.
    #[must_use]
    pub fn with<P>(mut self, parameter: &super::Parameter<P>, value: P) -> Self {
        self.insert(parameter, value);
        self
    }

    /// Inserts or replaces one typed parameter value.
    pub fn insert<P>(&mut self, parameter: &super::Parameter<P>, value: P) {
        self.values.insert(
            parameter.name().to_owned(),
            BoundParameter {
                ty: parameter.type_def(),
                datum: parameter.binding().to_datum(&value),
            },
        );
    }

    /// Number of currently bound parameter names.
    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Whether no parameters are bound.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Returns one bound parameter by its symbolic name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&BoundParameter> {
        self.values.get(name)
    }

    /// Iterates bound parameters in canonical name order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (&str, &BoundParameter)> {
        self.values
            .iter()
            .map(|(name, value)| (name.as_str(), value))
    }
}

/// Record values supplied in exactly the same scope order used for binding.
pub struct EvalContext<'a> {
    pub(crate) records: Vec<&'a dyn RecordView>,
    pub(crate) null_extended: Vec<bool>,
    pub(crate) parameters: Cow<'a, Parameters>,
}

impl<'a> Default for EvalContext<'a> {
    fn default() -> Self {
        Self {
            records: Vec::new(),
            null_extended: Vec::new(),
            parameters: Cow::Owned(Parameters::new()),
        }
    }
}

impl<'a> EvalContext<'a> {
    /// Creates an empty evaluation context.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a single-record evaluation context.
    #[must_use]
    pub fn single(record: &'a dyn RecordView) -> Self {
        Self {
            records: vec![record],
            null_extended: vec![false],
            parameters: Cow::Owned(Parameters::new()),
        }
    }

    /// Adds one record in scope order.
    #[must_use]
    pub fn with_record(mut self, record: &'a dyn RecordView) -> Self {
        self.records.push(record);
        self.null_extended.push(false);
        self
    }

    /// Adds one synthetic null-extended record scope in scope order.
    ///
    /// Execution engines use this for an unmatched side of an outer join. The
    /// provenance is consumed only when an expression has already been bound with
    /// an explicit nullable lift; ordinary records must use [`Self::with_record`].
    #[doc(hidden)]
    #[must_use]
    pub fn with_null_extended_record(mut self, record: &'a dyn RecordView) -> Self {
        self.records.push(record);
        self.null_extended.push(true);
        self
    }

    /// Reuses a canonical parameter set for this evaluation.
    #[must_use]
    pub fn with_parameters(mut self, parameters: &'a Parameters) -> Self {
        self.parameters = Cow::Borrowed(parameters);
        self
    }

    /// Binds a typed parameter value.
    #[must_use]
    pub fn with_parameter<P>(mut self, parameter: &super::Parameter<P>, value: P) -> Self {
        self.parameters.to_mut().insert(parameter, value);
        self
    }
}
