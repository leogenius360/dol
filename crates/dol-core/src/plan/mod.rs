//! Backend-independent logical-plan vocabulary produced by [`crate::Pipeline`].
//!
//! Plan identifiers and node topology are local compiler mechanics. Semantic
//! identity is carried by canonical fingerprints over model definitions,
//! prepared expressions, and logical operations rather than by local node IDs
//! or human-readable source aliases.

mod engine_view;
mod identity;
mod optimize;

use std::sync::Arc;

use crate::analytics::{AggregateKind, AggregateSelectionKind};
use crate::diagnostic::{Diagnostic, Result};
use crate::expr::{EvalContext, PreparedExpression, PreparedKind};
use crate::fingerprint::Fingerprint;
use crate::model::ModelDef;
use crate::types::TypeDef;

pub use engine_view::{
    LogicalBinaryOp, LogicalExprNodeKind, LogicalExprNodeView, LogicalExprView, LogicalUnaryOp,
};

use identity::fingerprint_plan;
pub(crate) use identity::{
    fingerprint_aggregate_stage, fingerprint_expression_stage, fingerprint_join_stage,
    fingerprint_projection_stage, fingerprint_set_stage, fingerprint_slice_stage,
    fingerprint_sort_stage, fingerprint_source, fingerprint_unary_stage, fingerprint_unnest_stage,
    fingerprint_window_stage,
};

/// A logical-plan-local node identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlanId(u32);

impl PlanId {
    #[doc(hidden)]
    pub fn from_index(index: usize) -> Result<Self> {
        let raw = u32::try_from(index).map_err(|_| {
            Diagnostic::error(
                "PIPELINE-LIMIT-001",
                "logical plan exceeds u32 node capacity",
            )
        })?;
        Ok(Self(raw))
    }

    /// Zero-based local node index.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// Prepared semantic expression retained by a logical-plan node.
#[derive(Debug, Clone)]
pub struct LogicalExpr {
    pub(crate) prepared: PreparedExpression,
}

impl LogicalExpr {
    pub(crate) fn from_prepared(prepared: PreparedExpression) -> Self {
        Self { prepared }
    }

    /// Exact semantic result type.
    #[must_use]
    pub const fn type_def(&self) -> &TypeDef {
        &self.prepared.ty
    }

    /// Canonical pipeline-bound expression fingerprint.
    ///
    /// This includes source occurrence binding while remaining independent of
    /// human-readable source alias names and local scope identifiers.
    #[must_use]
    pub const fn bound_fingerprint(&self) -> Fingerprint {
        self.prepared.bound_fingerprint
    }

    /// Canonical normalized expression fingerprint before pipeline binding.
    #[must_use]
    pub const fn expression_fingerprint(&self) -> Fingerprint {
        self.prepared.expression_fingerprint
    }

    /// Number of flat prepared expression nodes.
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.prepared.nodes.len()
    }

    /// Number of model scopes captured by this expression.
    #[must_use]
    pub fn scope_count(&self) -> usize {
        self.prepared.scopes.len()
    }

    /// Number of leading expression scopes supplied by an enclosing pipeline.
    ///
    /// A non-zero value identifies a correlated expression. Remaining scopes
    /// belong to the pipeline node that owns this expression.
    #[must_use]
    pub const fn outer_scope_count(&self) -> usize {
        self.prepared.outer_scope_count
    }

    /// Logical plan nodes referenced by existential predicates in this expression.
    #[must_use]
    pub fn existential_dependencies(&self) -> Vec<PlanId> {
        let mut dependencies = Vec::new();
        collect_existential_dependencies(&self.prepared, &mut dependencies);
        dependencies
    }

    /// Evaluates this already-bound expression using DOL's normative local semantics.
    ///
    /// Existential expressions require a pipeline execution context and therefore
    /// return `EXPR-EVAL-112` through this direct evaluator.
    #[doc(hidden)]
    pub fn evaluate_datum(&self, context: &EvalContext<'_>) -> Result<crate::value::Datum> {
        self.prepared.evaluate_datum(context)
    }

    /// Evaluates this expression while resolving existential plan dependencies.
    ///
    /// Engine adapters use this hidden boundary to preserve the normative expression
    /// evaluator while supplying execution semantics for `exists()`.
    #[doc(hidden)]
    pub fn evaluate_datum_with_exists(
        &self,
        context: &EvalContext<'_>,
        resolve_exists: &mut dyn FnMut(PlanId) -> Result<crate::semantics::Truth>,
    ) -> Result<crate::value::Datum> {
        self.prepared
            .evaluate_datum_with_exists(context, resolve_exists)
    }

    pub(crate) fn remap_plan_ids(&mut self, remap: &[PlanId]) -> Result<()> {
        remap_expression_plan_ids(&mut self.prepared, remap)
    }

    pub(crate) fn direct_field(&self) -> Option<(usize, usize)> {
        let node = self.prepared.nodes.get(self.prepared.root.index())?;
        match &node.kind {
            PreparedKind::Field { scope, slot, .. } => Some((scope.index(), slot.index())),
            _ => None,
        }
    }
}

fn collect_existential_dependencies(expression: &PreparedExpression, output: &mut Vec<PlanId>) {
    for node in &expression.nodes {
        if let PreparedKind::Exists { subquery, .. } = &node.kind {
            output.push(*subquery);
        }
    }
}

fn remap_expression_plan_ids(expression: &mut PreparedExpression, remap: &[PlanId]) -> Result<()> {
    for node in &mut expression.nodes {
        if let PreparedKind::Exists { subquery, .. } = &mut node.kind {
            *subquery = remap.get(subquery.index()).copied().ok_or_else(|| {
                Diagnostic::error(
                    "PIPELINE-OPT-001",
                    "optimizer encountered a non-topological existential plan reference",
                )
            })?;
        }
    }
    Ok(())
}

/// Structural form of a logical projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProjectionKind {
    /// One scalar or otherwise single symbolic value.
    Value,
    /// Ordered heterogeneous tuple.
    Tuple,
    /// Named record whose declaration order is non-semantic.
    Record,
}

/// Prepared scalar, tuple, or named-record projection.
#[derive(Debug, Clone)]
pub struct LogicalProjection {
    kind: ProjectionKind,
    expressions: Box<[LogicalExpr]>,
    names: Box<[Arc<str>]>,
    ty: TypeDef,
}

impl LogicalProjection {
    pub(crate) fn new(
        kind: ProjectionKind,
        expressions: Box<[LogicalExpr]>,
        names: Box<[Arc<str>]>,
        ty: TypeDef,
    ) -> Self {
        Self {
            kind,
            expressions,
            names,
            ty,
        }
    }

    /// Structural projection form.
    #[must_use]
    pub const fn kind(&self) -> ProjectionKind {
        self.kind
    }

    /// Prepared component expressions in semantic projection order.
    #[must_use]
    pub fn expressions(&self) -> &[LogicalExpr] {
        &self.expressions
    }

    /// Record field names in the same order as [`Self::expressions`].
    ///
    /// Scalar and tuple projections return an empty slice.
    #[must_use]
    pub fn names(&self) -> &[Arc<str>] {
        &self.names
    }

    /// Exact semantic output type.
    #[must_use]
    pub const fn type_def(&self) -> &TypeDef {
        &self.ty
    }
}

/// One prepared aggregate operation in a logical reduction.
#[derive(Debug, Clone)]
pub struct LogicalAggregate {
    kind: AggregateKind,
    input: Option<LogicalExpr>,
    ty: TypeDef,
}

impl LogicalAggregate {
    pub(crate) fn new(kind: AggregateKind, input: Option<LogicalExpr>, ty: TypeDef) -> Self {
        Self { kind, input, ty }
    }

    /// Aggregate operation.
    #[must_use]
    pub const fn kind(&self) -> AggregateKind {
        self.kind
    }

    /// Prepared input expression, absent only for row count.
    #[must_use]
    pub const fn input(&self) -> Option<&LogicalExpr> {
        self.input.as_ref()
    }

    /// Exact semantic result type of this aggregate.
    #[must_use]
    pub const fn type_def(&self) -> &TypeDef {
        &self.ty
    }
}

/// Structural form and exact output type of one aggregate selection.
#[derive(Debug, Clone)]
pub struct LogicalAggregateSelection {
    kind: AggregateSelectionKind,
    aggregates: Box<[LogicalAggregate]>,
    ty: TypeDef,
}

impl LogicalAggregateSelection {
    pub(crate) fn new(
        kind: AggregateSelectionKind,
        aggregates: Box<[LogicalAggregate]>,
        ty: TypeDef,
    ) -> Self {
        Self {
            kind,
            aggregates,
            ty,
        }
    }

    /// Whether the result is one value or an ordered tuple.
    #[must_use]
    pub const fn kind(&self) -> AggregateSelectionKind {
        self.kind
    }

    /// Prepared aggregate operations in result order.
    #[must_use]
    pub fn aggregates(&self) -> &[LogicalAggregate] {
        &self.aggregates
    }

    /// Exact semantic output type.
    #[must_use]
    pub const fn type_def(&self) -> &TypeDef {
        &self.ty
    }
}

/// Ordering direction for one logical sort expression.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SortDirection {
    /// Ascending semantic order.
    Ascending,
    /// Descending semantic order.
    Descending,
}

/// One prepared logical sort expression.
#[derive(Debug, Clone)]
pub struct SortExpr {
    expression: LogicalExpr,
    direction: SortDirection,
}

impl SortExpr {
    pub(crate) fn new(expression: LogicalExpr, direction: SortDirection) -> Self {
        Self {
            expression,
            direction,
        }
    }

    /// Prepared expression used as the sort key.
    #[must_use]
    pub const fn expression(&self) -> &LogicalExpr {
        &self.expression
    }

    /// Requested ordering direction.
    #[must_use]
    pub const fn direction(&self) -> SortDirection {
        self.direction
    }
}

/// Logical join semantics supported by the current pipeline phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JoinKind {
    /// Retains pairs for which the join condition evaluates to `True`.
    Inner,
    /// Preserves every left row and lifts unmatched right rows to nullable output.
    Left,
    /// Preserves every right row and lifts unmatched left rows to nullable output.
    Right,
    /// Preserves rows from both sides and lifts each unmatched side to nullable output.
    Full,
    /// Cartesian product with no join condition.
    Cross,
}

/// Logical set operation over two pipelines with identical semantic output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SetOperator {
    /// Set union with duplicate elimination.
    Union,
    /// Bag union retaining duplicates.
    UnionAll,
    /// Set intersection.
    Intersect,
    /// Set difference (`left - right`).
    Except,
}

impl SetOperator {
    pub(crate) const fn requires_equality(self) -> bool {
        !matches!(self, Self::UnionAll)
    }
}

/// Window function represented by one logical window stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WindowFunctionKind {
    /// One-based row position within each ordered partition.
    RowNumber,
    /// One-based rank with gaps between peer groups.
    Rank,
    /// One-based rank without gaps between peer groups.
    DenseRank,
    /// Checked exact-numeric sum over the selected rows frame.
    Sum,
}

/// One boundary of an explicit row-count window frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WindowFrameBound {
    /// The first row in the partition.
    UnboundedPreceding,
    /// `n` rows before the current row.
    Preceding(u64),
    /// The current row.
    CurrentRow,
    /// `n` rows after the current row.
    Following(u64),
    /// The last row in the partition.
    UnboundedFollowing,
}

/// Explicit `ROWS BETWEEN ... AND ...` semantics for aggregate windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RowsFrame {
    start: WindowFrameBound,
    end: WindowFrameBound,
}

impl RowsFrame {
    /// Creates an explicit row-count frame. Invalid boundary ordering is rejected during planning.
    #[must_use]
    pub const fn new(start: WindowFrameBound, end: WindowFrameBound) -> Self {
        Self {
            start: canonical_frame_bound(start),
            end: canonical_frame_bound(end),
        }
    }

    /// Entire partition, from the first row through the last row.
    #[must_use]
    pub const fn all() -> Self {
        Self::new(
            WindowFrameBound::UnboundedPreceding,
            WindowFrameBound::UnboundedFollowing,
        )
    }

    /// Running frame from the first row through the current row.
    #[must_use]
    pub const fn to_current() -> Self {
        Self::new(
            WindowFrameBound::UnboundedPreceding,
            WindowFrameBound::CurrentRow,
        )
    }

    /// Symmetric frame containing at most `radius` preceding/following rows around the current row.
    #[must_use]
    pub const fn around(radius: u64) -> Self {
        Self::new(
            WindowFrameBound::Preceding(radius),
            WindowFrameBound::Following(radius),
        )
    }

    /// Frame start boundary.
    #[must_use]
    pub const fn start(self) -> WindowFrameBound {
        self.start
    }

    /// Frame end boundary.
    #[must_use]
    pub const fn end(self) -> WindowFrameBound {
        self.end
    }
}

const fn canonical_frame_bound(bound: WindowFrameBound) -> WindowFrameBound {
    match bound {
        WindowFrameBound::Preceding(0) | WindowFrameBound::Following(0) => {
            WindowFrameBound::CurrentRow
        }
        bound => bound,
    }
}

/// Prepared window function together with partition/order/frame semantics.
#[derive(Debug, Clone)]
pub struct LogicalWindow {
    kind: WindowFunctionKind,
    input: Option<LogicalExpr>,
    partition: Option<Box<LogicalProjection>>,
    order: Box<[SortExpr]>,
    frame: Option<RowsFrame>,
    ty: TypeDef,
}

pub(crate) struct LogicalWindowSpec {
    pub(crate) kind: WindowFunctionKind,
    pub(crate) input: Option<LogicalExpr>,
    pub(crate) partition: Option<Box<LogicalProjection>>,
    pub(crate) order: Box<[SortExpr]>,
    pub(crate) frame: Option<RowsFrame>,
    pub(crate) ty: TypeDef,
}

impl LogicalWindow {
    pub(crate) fn new(spec: LogicalWindowSpec) -> Self {
        Self {
            kind: spec.kind,
            input: spec.input,
            partition: spec.partition,
            order: spec.order,
            frame: spec.frame,
            ty: spec.ty,
        }
    }

    /// Window function operation.
    #[must_use]
    pub const fn kind(&self) -> WindowFunctionKind {
        self.kind
    }

    /// Optional value input used by aggregate-style windows.
    #[must_use]
    pub const fn input(&self) -> Option<&LogicalExpr> {
        self.input.as_ref()
    }

    /// Optional partition key projection.
    #[must_use]
    pub fn partition(&self) -> Option<&LogicalProjection> {
        self.partition.as_deref()
    }

    /// Ordered peer/row ordering expressions.
    #[must_use]
    pub fn order(&self) -> &[SortExpr] {
        &self.order
    }

    /// Explicit rows frame, present for aggregate-style windows.
    #[must_use]
    pub const fn frame(&self) -> Option<RowsFrame> {
        self.frame
    }

    /// Exact semantic result type.
    #[must_use]
    pub const fn type_def(&self) -> &TypeDef {
        &self.ty
    }
}

/// Semantic output shape of one logical pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanOutput {
    /// Rows retain the complete semantic model shape.
    Model(Box<ModelDef>),
    /// Rows are values produced by one projection expression.
    Value(TypeDef),
    /// Rows are the ordered product of multiple pipeline outputs.
    Product(Box<[PlanOutput]>),
    /// Rows may be absent because this output originated on an outer-join side.
    Nullable(Box<PlanOutput>),
}

/// One backend-independent logical operation.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum LogicalNode {
    /// Storage-independent semantic model source.
    Source {
        /// Complete immutable semantic model definition.
        model: Box<ModelDef>,
        /// Optional local source alias used only for expression binding.
        alias: Option<Arc<str>>,
    },
    /// Retains rows whose truth expression evaluates to `True`.
    Filter {
        /// Input plan node.
        input: PlanId,
        /// Bound truth-valued expression.
        condition: Box<LogicalExpr>,
    },
    /// Computes one typed scalar, tuple, or named record per input row.
    Project {
        /// Input plan node.
        input: PlanId,
        /// Bound typed projection.
        projection: Box<LogicalProjection>,
    },
    /// Reduces the input globally or by one typed grouping projection.
    Aggregate {
        /// Input plan node.
        input: PlanId,
        /// Optional grouping key; absent for one global result.
        groups: Option<Box<LogicalProjection>>,
        /// Aggregate result selection.
        aggregates: Box<LogicalAggregateSelection>,
    },
    /// Expands one list-valued input row into zero or more element rows.
    Unnest {
        /// List-valued input plan node.
        input: PlanId,
        /// Exact semantic element type emitted by the stage.
        element: TypeDef,
    },
    /// Computes one window value per input row while preserving the input row.
    Window {
        /// Input plan node.
        input: PlanId,
        /// Prepared window semantics.
        window: Box<LogicalWindow>,
    },
    /// Orders rows by one or more semantic expressions.
    Sort {
        /// Input plan node.
        input: PlanId,
        /// Ordered sort keys.
        keys: Box<[SortExpr]>,
    },
    /// Removes duplicate output rows using DOL equality semantics.
    Distinct {
        /// Input plan node.
        input: PlanId,
    },
    /// Applies logical offset and/or limit.
    Slice {
        /// Input plan node.
        input: PlanId,
        /// Number of rows skipped before output.
        offset: u64,
        /// Maximum rows produced after the offset.
        limit: Option<u64>,
    },
    /// Combines two logical inputs without introducing a public binary-pipeline type.
    Join {
        /// Left logical input.
        left: PlanId,
        /// Right logical input.
        right: PlanId,
        /// Join semantics.
        kind: JoinKind,
        /// Bound condition for conditional joins; absent for cross joins.
        condition: Option<Box<LogicalExpr>>,
    },
    /// Combines two pipelines with the same semantic output using set algebra.
    Set {
        /// Left logical input.
        left: PlanId,
        /// Right logical input.
        right: PlanId,
        /// Set operation.
        operator: SetOperator,
    },
}

/// Validated backend-independent logical pipeline.
#[derive(Debug, Clone)]
pub struct LogicalPlan {
    nodes: Box<[LogicalNode]>,
    root: PlanId,
    output: PlanOutput,
    fingerprint: Fingerprint,
}

impl LogicalPlan {
    pub(crate) fn new(
        nodes: Vec<LogicalNode>,
        root: PlanId,
        output: PlanOutput,
        root_fingerprint: Fingerprint,
    ) -> Self {
        let fingerprint = fingerprint_plan(root_fingerprint, &output);
        Self {
            nodes: nodes.into_boxed_slice(),
            root,
            output,
            fingerprint,
        }
    }

    /// Plan nodes in topological dependency order.
    #[must_use]
    pub fn nodes(&self) -> &[LogicalNode] {
        &self.nodes
    }

    /// Root node producing the plan result.
    #[must_use]
    pub const fn root(&self) -> PlanId {
        self.root
    }

    /// Semantic output shape.
    #[must_use]
    pub const fn output(&self) -> &PlanOutput {
        &self.output
    }

    /// Canonical logical-plan fingerprint independent of local node IDs and aliases.
    #[must_use]
    pub const fn fingerprint(&self) -> Fingerprint {
        self.fingerprint
    }

    /// Returns a conservatively optimized equivalent logical plan.
    ///
    /// Rewrites are restricted to transformations whose DOL semantics are
    /// invariant under the rewrite. The canonical semantic fingerprint is
    /// therefore preserved even when redundant logical nodes are removed.
    pub fn optimized(&self) -> Result<Self> {
        optimize::optimize(self)
    }

    pub(crate) fn from_optimized(
        nodes: Vec<LogicalNode>,
        root: PlanId,
        output: PlanOutput,
        fingerprint: Fingerprint,
    ) -> Self {
        Self {
            nodes: nodes.into_boxed_slice(),
            root,
            output,
            fingerprint,
        }
    }
}

#[derive(Default)]
pub(crate) struct PlanBuilder {
    nodes: Vec<LogicalNode>,
    fingerprints: Vec<Fingerprint>,
}

impl PlanBuilder {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn push(&mut self, node: LogicalNode, fingerprint: Fingerprint) -> Result<PlanId> {
        let id = PlanId::from_index(self.nodes.len())?;
        self.nodes.push(node);
        self.fingerprints.push(fingerprint);
        Ok(id)
    }

    pub(crate) fn fingerprint(&self, id: PlanId) -> Result<Fingerprint> {
        self.fingerprints.get(id.index()).copied().ok_or_else(|| {
            Diagnostic::error(
                "PIPELINE-PLAN-001",
                "logical plan references an invalid local node identifier",
            )
        })
    }

    pub(crate) fn finish(self, root: PlanId, output: PlanOutput) -> Result<LogicalPlan> {
        let root_fingerprint = self.fingerprint(root)?;
        Ok(LogicalPlan::new(self.nodes, root, output, root_fingerprint))
    }
}
