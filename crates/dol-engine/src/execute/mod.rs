//! Engine identity, execution requests, limits, and executable SPI traits.

use core::time::Duration;

use dol_core::diagnostic::{Diagnostic, Result};
use dol_core::expr::Parameters;
use dol_core::ops::{LogicalWrite, WriteOutcome};
use dol_core::plan::{LogicalNode, LogicalPlan};

use crate::capability::{Capabilities, Support};
use crate::placement::PlacementPolicy;
use crate::stream::DataStream;

/// Human-readable engine identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineInfo {
    /// Stable engine kind, such as `memory`, `postgres`, or `mongodb`.
    pub kind: &'static str,
    /// Adapter implementation version.
    pub version: &'static str,
}

/// Baseline interface shared by all engines.
pub trait Engine: Send + Sync + 'static {
    /// Returns engine identity.
    fn info(&self) -> &EngineInfo;

    /// Returns exact-semantic capability claims.
    fn capabilities(&self) -> &Capabilities;

    /// Refines coarse operation-family capabilities for one concrete logical
    /// node. Engines should reject type/expression variants their compiler
    /// cannot reproduce exactly.
    fn support_for_node(&self, node: &LogicalNode) -> Support {
        crate::placement::capability_support_for_node(node, self.capabilities())
    }
}

/// Resource limits applied to one execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionLimits {
    /// Maximum logical rows materialized by any one engine stage.
    pub max_materialized_rows: Option<u64>,
    /// Maximum logical row payload bytes materialized by any one engine stage.
    pub max_materialized_bytes: Option<u64>,
    /// Maximum logical plan nodes.
    pub max_plan_nodes: usize,
    /// Maximum retained expression nodes across the plan.
    pub max_expression_nodes: usize,
    /// Maximum concurrent backend work units.
    pub max_concurrency: usize,
    /// Optional execution timeout.
    pub timeout: Option<Duration>,
}

impl ExecutionLimits {
    /// Validates plan-level limits before physical execution begins.
    pub fn validate_plan(&self, plan: &LogicalPlan) -> Result<()> {
        if plan.nodes().len() > self.max_plan_nodes {
            return Err(Diagnostic::error(
                "ENGINE-LIMIT-001",
                "logical plan exceeds the configured execution node limit",
            ));
        }
        let expression_nodes = plan
            .nodes()
            .iter()
            .map(logical_node_expression_count)
            .sum::<usize>();
        if expression_nodes > self.max_expression_nodes {
            return Err(Diagnostic::error(
                "ENGINE-LIMIT-002",
                "logical plan exceeds the configured execution expression limit",
            ));
        }
        if self.max_concurrency == 0 {
            return Err(Diagnostic::error(
                "ENGINE-LIMIT-003",
                "execution concurrency limit must be greater than zero",
            ));
        }
        Ok(())
    }
}

impl Default for ExecutionLimits {
    fn default() -> Self {
        Self {
            max_materialized_rows: Some(100_000),
            max_materialized_bytes: Some(64 * 1024 * 1024),
            max_plan_nodes: 16_384,
            max_expression_nodes: 65_536,
            max_concurrency: 8,
            timeout: None,
        }
    }
}

/// Resource limits applied to one logical write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteLimits {
    /// Maximum rows one write may affect atomically.
    pub max_affected_rows: Option<u64>,
    /// Maximum concurrent backend work units used by the write.
    pub max_concurrency: usize,
    /// Optional write timeout.
    pub timeout: Option<Duration>,
}

impl WriteLimits {
    /// Validates write-level limits before execution begins.
    pub fn validate(&self) -> Result<()> {
        if self.max_concurrency == 0 {
            return Err(Diagnostic::error(
                "ENGINE-LIMIT-003",
                "write concurrency limit must be greater than zero",
            ));
        }
        Ok(())
    }
}

impl Default for WriteLimits {
    fn default() -> Self {
        Self {
            max_affected_rows: Some(100_000),
            max_concurrency: 8,
            timeout: None,
        }
    }
}

/// Placement, resource, and batching policy for one plan execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionOptions {
    /// Resource limits enforced by the engine/adapter.
    pub limits: ExecutionLimits,
    /// Remote-only or explicitly bounded hybrid placement policy.
    pub placement: PlacementPolicy,
    /// Preferred result batch size for pull streams.
    pub batch_rows: usize,
}

impl Default for ExecutionOptions {
    fn default() -> Self {
        Self {
            limits: ExecutionLimits::default(),
            placement: PlacementPolicy::default(),
            batch_rows: 1_024,
        }
    }
}

/// Borrowed request passed to a logical-plan executor.
pub struct ExecutionRequest<'a> {
    /// Validated backend-independent logical plan.
    pub plan: &'a LogicalPlan,
    /// Typed parameter bindings kept separate from plan identity.
    pub parameters: &'a Parameters,
    /// Placement/resource/batching policy.
    pub options: &'a ExecutionOptions,
}

/// Engine capable of executing logical pipeline plans.
pub trait PlanExecutor: Engine {
    /// Pull stream produced by this engine.
    type Stream: DataStream;

    /// Executes one validated logical plan.
    fn execute_plan(&self, request: ExecutionRequest<'_>) -> Result<Self::Stream>;
}

/// Borrowed request passed to a write executor.
pub struct WriteRequest<'a> {
    /// Validated backend-independent logical write.
    pub write: &'a LogicalWrite,
    /// Typed parameter bindings separated from write identity.
    pub parameters: &'a Parameters,
    /// Resource limits for the write.
    pub limits: &'a WriteLimits,
}

/// Engine capable of applying first-class logical writes.
pub trait WriteExecutor: Engine {
    /// Applies one write atomically according to the engine's advertised semantics.
    fn execute_write(&mut self, request: WriteRequest<'_>) -> Result<WriteOutcome>;
}

fn logical_node_expression_count(node: &LogicalNode) -> usize {
    match node {
        LogicalNode::Source { .. }
        | LogicalNode::Unnest { .. }
        | LogicalNode::Distinct { .. }
        | LogicalNode::Slice { .. }
        | LogicalNode::Set { .. } => 0,
        LogicalNode::Filter { condition, .. } => condition.node_count(),
        LogicalNode::Project { projection, .. } => projection
            .expressions()
            .iter()
            .map(dol_core::plan::LogicalExpr::node_count)
            .sum(),
        LogicalNode::Aggregate {
            groups, aggregates, ..
        } => {
            let groups = groups.as_ref().map_or(0, |projection| {
                projection
                    .expressions()
                    .iter()
                    .map(dol_core::plan::LogicalExpr::node_count)
                    .sum()
            });
            let aggregates = aggregates
                .aggregates()
                .iter()
                .filter_map(dol_core::plan::LogicalAggregate::input)
                .map(dol_core::plan::LogicalExpr::node_count)
                .sum::<usize>();
            groups + aggregates
        }
        LogicalNode::Window { window, .. } => {
            let input = window
                .input()
                .map_or(0, dol_core::plan::LogicalExpr::node_count);
            let partition = window.partition().map_or(0, |projection| {
                projection
                    .expressions()
                    .iter()
                    .map(dol_core::plan::LogicalExpr::node_count)
                    .sum()
            });
            let order = window
                .order()
                .iter()
                .map(|sort| sort.expression().node_count())
                .sum::<usize>();
            input + partition + order
        }
        LogicalNode::Sort { keys, .. } => {
            keys.iter().map(|sort| sort.expression().node_count()).sum()
        }
        LogicalNode::Join { condition, .. } => condition
            .as_ref()
            .map_or(0, |condition| condition.node_count()),
        _ => 0,
    }
}
