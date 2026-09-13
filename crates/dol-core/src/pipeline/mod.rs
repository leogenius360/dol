//! Declarative data-computation pipelines.
//!
//! [`Pipeline<T>`] is DOL's primary symbolic computation over zero or more
//! values. Method chaining expresses semantic dependency, not a mandatory
//! physical execution order; validated pipelines lower to a logical DAG that
//! engines may optimize before execution.

mod authoring_identity;
mod compile;
mod projection;
mod window;

use core::fmt;
use core::marker::PhantomData;
use std::collections::HashSet;
use std::sync::{Arc, OnceLock};

use crate::analytics::{AggregateSelection, AggregateSelectionSpec};
use crate::diagnostic::Result;
use crate::expr::{Expr, ExprSource, ExpressionSpec};
use crate::fingerprint::Fingerprint;
use crate::limits::PipelineLimits;
use crate::model::{Model, ModelDef};
use crate::plan::{JoinKind, LogicalPlan, SetOperator, SortDirection};
use crate::semantics::Truth;
use crate::types::DataType;

pub use crate::plan::{ProjectionKind, RowsFrame, WindowFrameBound};
pub use projection::{ProjectionRecord, ProjectionSource, ProjectionTuple, RecordProjection};
pub use window::{Window, dense_rank, rank, row_number, window_sum, window_sum_present};

#[derive(Clone)]
pub(crate) struct ModelSource {
    resolve: fn() -> Result<&'static ModelDef>,
    alias: Option<Arc<str>>,
}

impl ModelSource {
    fn of<M: Model>(alias: Option<Arc<str>>) -> Self {
        Self {
            resolve: M::model_def,
            alias,
        }
    }

    pub(crate) fn resolve(&self) -> Result<&'static ModelDef> {
        (self.resolve)()
    }

    pub(crate) fn alias(&self) -> Option<&Arc<str>> {
        self.alias.as_ref()
    }
}

pub(crate) struct PipelineNode {
    kind: PipelineKind,
    default_plan: OnceLock<Arc<LogicalPlan>>,
}

#[derive(Clone)]
pub(crate) enum PipelineKind {
    Source(ModelSource),
    Filter {
        input: Arc<PipelineNode>,
        condition: ExpressionSpec,
    },
    Project {
        input: Arc<PipelineNode>,
        projection: projection::ProjectionSpec,
    },
    Aggregate {
        input: Arc<PipelineNode>,
        groups: Option<projection::ProjectionSpec>,
        aggregates: AggregateSelectionSpec,
    },
    Unnest {
        input: Arc<PipelineNode>,
    },
    Window {
        input: Arc<PipelineNode>,
        window: Box<window::WindowSpec>,
    },
    Sort {
        input: Arc<PipelineNode>,
        expression: ExpressionSpec,
        direction: SortDirection,
    },
    Distinct {
        input: Arc<PipelineNode>,
    },
    Slice {
        input: Arc<PipelineNode>,
        offset: u64,
        limit: Option<u64>,
    },
    Join {
        left: Arc<PipelineNode>,
        right: Arc<PipelineNode>,
        kind: JoinKind,
        condition: Option<ExpressionSpec>,
    },
    Set {
        left: Arc<PipelineNode>,
        right: Arc<PipelineNode>,
        operator: SetOperator,
    },
}

impl PipelineNode {
    fn new(kind: PipelineKind) -> Arc<Self> {
        Arc::new(Self {
            kind,
            default_plan: OnceLock::new(),
        })
    }

    pub(crate) const fn kind(&self) -> &PipelineKind {
        &self.kind
    }

    fn cached_default_plan(&self) -> Option<Arc<LogicalPlan>> {
        self.default_plan.get().map(Arc::clone)
    }

    fn cache_default_plan(&self, plan: Arc<LogicalPlan>) -> Arc<LogicalPlan> {
        let _ = self.default_plan.set(Arc::clone(&plan));
        self.cached_default_plan().unwrap_or(plan)
    }
}

pub(crate) fn fingerprint_authoring_node(node: &Arc<PipelineNode>) -> Result<Fingerprint> {
    authoring_identity::fingerprint(node)
}

impl PipelineNode {
    pub(crate) fn push_structural_inputs(&self, output: &mut Vec<Arc<Self>>) {
        match &self.kind {
            PipelineKind::Source(_) => {}
            PipelineKind::Filter { input, .. } => output.push(Arc::clone(input)),
            PipelineKind::Project { input, .. }
            | PipelineKind::Aggregate { input, .. }
            | PipelineKind::Unnest { input }
            | PipelineKind::Window { input, .. }
            | PipelineKind::Sort { input, .. }
            | PipelineKind::Distinct { input }
            | PipelineKind::Slice { input, .. } => output.push(Arc::clone(input)),
            PipelineKind::Join { left, right, .. } | PipelineKind::Set { left, right, .. } => {
                output.push(Arc::clone(right));
                output.push(Arc::clone(left));
            }
        }
    }

    fn push_all_inputs(&self, output: &mut Vec<Arc<Self>>) {
        self.push_structural_inputs(output);
        if let PipelineKind::Filter { condition, .. } = &self.kind {
            condition.push_pipeline_inputs(output);
        }
    }

    fn stage_count(root: &Arc<Self>) -> usize {
        let mut seen = HashSet::new();
        let mut stack = vec![Arc::clone(root)];
        while let Some(node) = stack.pop() {
            let pointer = Arc::as_ptr(&node);
            if !seen.insert(pointer) {
                continue;
            }
            node.push_all_inputs(&mut stack);
        }
        seen.len()
    }
}

/// Immutable declarative data computation producing zero or more `T` values.
///
/// A pipeline contains symbolic semantics only. It does not contain result
/// rows, own a database connection, or prescribe a physical execution order.
pub struct Pipeline<T> {
    pub(crate) node: Arc<PipelineNode>,
    _marker: PhantomData<fn() -> T>,
}

impl<T> Clone for Pipeline<T> {
    fn clone(&self) -> Self {
        Self {
            node: Arc::clone(&self.node),
            _marker: PhantomData,
        }
    }
}

impl<T> fmt::Debug for Pipeline<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pipeline")
            .field("stage_count", &self.stage_count())
            .finish_non_exhaustive()
    }
}

impl<M: Model> Pipeline<M> {
    /// Creates a storage-independent semantic source for model `M`.
    #[must_use]
    pub fn from_model() -> Self {
        Self::from_source_alias(None)
    }

    /// Creates a semantic model source with an explicit local binding alias.
    ///
    /// Aliases are required when the same model appears more than once in one
    /// expression scope. They are diagnostic/binding names and are not part of
    /// canonical pipeline identity.
    #[must_use]
    pub fn from_model_as(alias: impl Into<Arc<str>>) -> Self {
        Self::from_source_alias(Some(alias.into()))
    }

    fn from_source_alias(alias: Option<Arc<str>>) -> Self {
        Self {
            node: PipelineNode::new(PipelineKind::Source(ModelSource::of::<M>(alias))),
            _marker: PhantomData,
        }
    }
}

impl<T> Pipeline<T> {
    /// Adds a three-valued filter condition.
    ///
    /// Only rows for which the expression evaluates to [`Truth::True`] are
    /// retained. Adjacent filters canonicalize to one logical conjunction. In
    /// an existential subpipeline, field references supplied by an enclosing
    /// pipeline are bound as outer scopes when [`Self::exists`] is prepared.
    #[must_use]
    pub fn filter(self, condition: Expr<Truth>) -> Self {
        let condition = condition.spec().canonical_truth();
        let node = match self.node.kind() {
            PipelineKind::Filter {
                input,
                condition: existing,
            } => PipelineNode::new(PipelineKind::Filter {
                input: Arc::clone(input),
                condition: existing.clone().and_truth(condition),
            }),
            _ => PipelineNode::new(PipelineKind::Filter {
                input: self.node,
                condition,
            }),
        };
        Self {
            node,
            _marker: PhantomData,
        }
    }

    /// Projects every input row into a typed scalar, tuple, or named record.
    #[must_use]
    pub fn select<P>(self, projection: P) -> Pipeline<P::Value>
    where
        P: ProjectionSource,
    {
        Pipeline {
            node: PipelineNode::new(PipelineKind::Project {
                input: self.node,
                projection: projection.__projection_spec(),
            }),
            _marker: PhantomData,
        }
    }

    /// Reduces the entire pipeline to one typed aggregate result row.
    ///
    /// An empty input still produces one global aggregate row. Aggregate
    /// descriptors carry semantic behavior only; execution remains engine-owned.
    #[must_use]
    pub fn aggregate<A>(self, aggregates: A) -> Pipeline<A::Value>
    where
        A: AggregateSelection,
    {
        Pipeline {
            node: PipelineNode::new(PipelineKind::Aggregate {
                input: self.node,
                groups: None,
                aggregates: aggregates.__aggregate_selection(),
            }),
            _marker: PhantomData,
        }
    }

    /// Groups rows by a typed projection and reduces each group.
    ///
    /// This is one complete pipeline operation rather than a transient
    /// `GroupedPipeline` state, keeping grouping within the primary
    /// [`Pipeline`] abstraction.
    #[must_use]
    pub fn aggregate_by<K, A>(self, keys: K, aggregates: A) -> Pipeline<(K::Value, A::Value)>
    where
        K: ProjectionSource,
        A: AggregateSelection,
    {
        Pipeline {
            node: PipelineNode::new(PipelineKind::Aggregate {
                input: self.node,
                groups: Some(keys.__projection_spec()),
                aggregates: aggregates.__aggregate_selection(),
            }),
            _marker: PhantomData,
        }
    }

    /// Orders rows by one ascending semantic expression.
    #[must_use]
    pub fn order_by<S>(self, expression: S) -> Self
    where
        S: ExprSource,
    {
        self.sort(expression, SortDirection::Ascending)
    }

    /// Orders rows by one descending semantic expression.
    #[must_use]
    pub fn order_by_desc<S>(self, expression: S) -> Self
    where
        S: ExprSource,
    {
        self.sort(expression, SortDirection::Descending)
    }

    /// Removes duplicate output rows using DOL equality semantics.
    #[must_use]
    pub fn distinct(self) -> Self {
        Self {
            node: PipelineNode::new(PipelineKind::Distinct { input: self.node }),
            _marker: PhantomData,
        }
    }

    /// Skips the first `offset` rows of the logical result.
    #[must_use]
    pub fn offset(self, offset: u64) -> Self {
        Self {
            node: PipelineNode::new(PipelineKind::Slice {
                input: self.node,
                offset,
                limit: None,
            }),
            _marker: PhantomData,
        }
    }

    /// Limits the logical result to at most `limit` rows.
    #[must_use]
    pub fn limit(self, limit: u64) -> Self {
        Self {
            node: PipelineNode::new(PipelineKind::Slice {
                input: self.node,
                offset: 0,
                limit: Some(limit),
            }),
            _marker: PhantomData,
        }
    }

    /// Applies one logical offset and limit together.
    #[must_use]
    pub fn slice(self, offset: u64, limit: u64) -> Self {
        Self {
            node: PipelineNode::new(PipelineKind::Slice {
                input: self.node,
                offset,
                limit: Some(limit),
            }),
            _marker: PhantomData,
        }
    }

    /// Inner-joins this pipeline with another pipeline.
    ///
    /// The result remains one [`Pipeline`] and is typed as the ordered product
    /// `(T, U)`. No topology-specific public pipeline type is introduced.
    #[must_use]
    pub fn join<U>(self, other: Pipeline<U>, condition: Expr<Truth>) -> Pipeline<(T, U)> {
        Pipeline {
            node: PipelineNode::new(PipelineKind::Join {
                left: self.node,
                right: other.node,
                kind: JoinKind::Inner,
                condition: Some(condition.spec()),
            }),
            _marker: PhantomData,
        }
    }

    /// Left-outer-joins this pipeline with another pipeline.
    ///
    /// Unmatched right rows are represented as `None`. Non-null fields from
    /// the right source must be referenced through [`crate::model::Field::nullable`] after
    /// this join.
    #[must_use]
    pub fn left_join<U>(
        self,
        other: Pipeline<U>,
        condition: Expr<Truth>,
    ) -> Pipeline<(T, Option<U>)> {
        Pipeline {
            node: PipelineNode::new(PipelineKind::Join {
                left: self.node,
                right: other.node,
                kind: JoinKind::Left,
                condition: Some(condition.spec()),
            }),
            _marker: PhantomData,
        }
    }

    /// Right-outer-joins this pipeline with another pipeline.
    ///
    /// Unmatched left rows are represented as `None`.
    #[must_use]
    pub fn right_join<U>(
        self,
        other: Pipeline<U>,
        condition: Expr<Truth>,
    ) -> Pipeline<(Option<T>, U)> {
        Pipeline {
            node: PipelineNode::new(PipelineKind::Join {
                left: self.node,
                right: other.node,
                kind: JoinKind::Right,
                condition: Some(condition.spec()),
            }),
            _marker: PhantomData,
        }
    }

    /// Full-outer-joins this pipeline with another pipeline.
    ///
    /// Each unmatched side is represented as `None`.
    #[must_use]
    pub fn full_join<U>(
        self,
        other: Pipeline<U>,
        condition: Expr<Truth>,
    ) -> Pipeline<(Option<T>, Option<U>)> {
        Pipeline {
            node: PipelineNode::new(PipelineKind::Join {
                left: self.node,
                right: other.node,
                kind: JoinKind::Full,
                condition: Some(condition.spec()),
            }),
            _marker: PhantomData,
        }
    }

    /// Forms the Cartesian product of this pipeline and another pipeline.
    #[must_use]
    pub fn cross_join<U>(self, other: Pipeline<U>) -> Pipeline<(T, U)> {
        Pipeline {
            node: PipelineNode::new(PipelineKind::Join {
                left: self.node,
                right: other.node,
                kind: JoinKind::Cross,
                condition: None,
            }),
            _marker: PhantomData,
        }
    }

    /// Converts this pipeline into a truth expression that is true when at least
    /// one row exists.
    ///
    /// The returned value is an ordinary [`Expr<Truth>`], so existential tests
    /// compose with normal boolean expressions and are consumed by the regular
    /// [`Pipeline::filter`] operation.
    #[must_use]
    pub fn exists(self) -> Expr<Truth> {
        Expr::<Truth>::existential(self.node)
    }

    /// Convenience negation of [`Self::exists`].
    #[must_use]
    pub fn not_exists(self) -> Expr<Truth> {
        self.exists().negate()
    }

    /// Computes one typed window value per input row.
    ///
    /// The input row is preserved as the first product component, so the
    /// resulting pipeline is `Pipeline<(T, W)>`. Source scopes also remain
    /// visible for subsequent window/filter/order operations.
    #[must_use]
    pub fn window<W>(self, window: Window<W>) -> Pipeline<(T, W)> {
        Pipeline {
            node: PipelineNode::new(PipelineKind::Window {
                input: self.node,
                window: Box::new(window.spec),
            }),
            _marker: PhantomData,
        }
    }

    /// Set union with duplicate elimination.
    #[must_use]
    pub fn union(self, other: Pipeline<T>) -> Self {
        self.set(other, SetOperator::Union)
    }

    /// Bag union retaining duplicates.
    #[must_use]
    pub fn union_all(self, other: Pipeline<T>) -> Self {
        self.set(other, SetOperator::UnionAll)
    }

    /// Set intersection.
    #[must_use]
    pub fn intersect(self, other: Pipeline<T>) -> Self {
        self.set(other, SetOperator::Intersect)
    }

    /// Set difference (`self - other`).
    #[must_use]
    pub fn except(self, other: Pipeline<T>) -> Self {
        self.set(other, SetOperator::Except)
    }

    /// Number of unique authoring nodes reachable from this pipeline root.
    #[must_use]
    pub fn stage_count(&self) -> usize {
        PipelineNode::stage_count(&self.node)
    }

    /// Returns the shared default-limit logical plan, preparing it once.
    ///
    /// Successful default-limit lowering is cached on the immutable pipeline
    /// root. Failed lowering is never cached, so diagnostics remain retryable.
    pub fn prepared_plan(&self) -> Result<Arc<LogicalPlan>> {
        if let Some(plan) = self.node.cached_default_plan() {
            return Ok(plan);
        }
        let plan = Arc::new(compile::compile(&self.node, PipelineLimits::default())?);
        Ok(self.node.cache_default_plan(plan))
    }

    /// Validates and lowers this symbolic computation to an owned logical DAG.
    ///
    /// This compatibility API clones the cached default plan. Use
    /// [`Self::prepared_plan`] on repeated engine paths to share it directly.
    pub fn logical_plan(&self) -> Result<LogicalPlan> {
        Ok((*self.prepared_plan()?).clone())
    }

    /// Validates and lowers this pipeline under explicit resource limits.
    pub fn logical_plan_with_limits(&self, limits: PipelineLimits) -> Result<LogicalPlan> {
        compile::compile(&self.node, limits)
    }

    /// Canonical fingerprint of the validated logical plan.
    pub fn fingerprint(&self) -> Result<Fingerprint> {
        Ok(self.prepared_plan()?.fingerprint())
    }

    fn sort<S>(self, expression: S, direction: SortDirection) -> Self
    where
        S: ExprSource,
    {
        let expression = expression.into_expression();
        Self {
            node: PipelineNode::new(PipelineKind::Sort {
                input: self.node,
                expression: expression.spec(),
                direction,
            }),
            _marker: PhantomData,
        }
    }

    fn set(self, other: Pipeline<T>, operator: SetOperator) -> Self {
        Self {
            node: PipelineNode::new(PipelineKind::Set {
                left: self.node,
                right: other.node,
                operator,
            }),
            _marker: PhantomData,
        }
    }
}

impl<U: DataType> Pipeline<Vec<U>> {
    /// Expands each list-valued row into its ordered element rows.
    ///
    /// Empty, missing, or null list values produce zero element rows. The
    /// resulting element type preserves element nullability exactly.
    #[must_use]
    pub fn unnest(self) -> Pipeline<U> {
        Pipeline {
            node: PipelineNode::new(PipelineKind::Unnest { input: self.node }),
            _marker: PhantomData,
        }
    }
}

impl<U: DataType> Pipeline<Option<Vec<U>>> {
    /// Expands concrete list values while treating null/missing lists as empty.
    #[must_use]
    pub fn unnest_present(self) -> Pipeline<U> {
        Pipeline {
            node: PipelineNode::new(PipelineKind::Unnest { input: self.node }),
            _marker: PhantomData,
        }
    }
}

#[cfg(all(test, target_arch = "x86_64"))]
mod performance_layout_contract {
    use super::PipelineNode;

    #[test]
    fn private_pipeline_node_layout_is_reviewed_on_x86_64() {
        assert_eq!(
            std::mem::size_of::<PipelineNode>(),
            280,
            "update perf/FOOTPRINT.md with measured impact before changing this expectation"
        );
        assert_eq!(
            std::mem::align_of::<PipelineNode>(),
            8,
            "update perf/FOOTPRINT.md with measured impact before changing this expectation"
        );
    }
}
