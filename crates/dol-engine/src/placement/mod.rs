//! Exact-capability placement analysis.

use dol_core::diagnostic::{Diagnostic, Result};
use dol_core::plan::{LogicalNode, LogicalPlan, PlanId};

use crate::capability::{Capabilities, Support};
use crate::execute::Engine;

/// Maximum data movement allowed for hybrid execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransferBudget {
    /// Maximum transferred rows.
    pub max_rows: u64,
    /// Maximum transferred bytes.
    pub max_bytes: u64,
    /// Optional maximum transferred batches.
    pub max_batches: Option<u64>,
}

impl TransferBudget {
    /// Validates that the budget can actually bound transfer.
    pub fn validate(self) -> Result<()> {
        if self.max_rows == 0 || self.max_bytes == 0 || self.max_batches == Some(0) {
            return Err(Diagnostic::error(
                "ENGINE-PLACEMENT-003",
                "hybrid transfer budgets must have non-zero row/byte/batch limits",
            ));
        }
        Ok(())
    }
}

/// Policy controlling whether a plan may leave residual local work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlacementPolicy {
    /// Reject plans that cannot be executed entirely by the assigned engine.
    #[default]
    RemoteOnly,
    /// Permit explicitly bounded transfer followed by local residual execution.
    Hybrid {
        /// Hard transfer budget that a hybrid executor must enforce.
        transfer: TransferBudget,
    },
}

/// Assigned location of one logical node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlacementSite {
    /// Execute through the assigned engine adapter.
    Engine,
    /// Requires local residual execution under a hybrid policy.
    LocalResidual,
}

/// Placement decision for one logical node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodePlacement {
    id: PlanId,
    site: PlacementSite,
    support: Support,
}

impl NodePlacement {
    /// Logical plan node identifier.
    #[must_use]
    pub const fn id(&self) -> PlanId {
        self.id
    }

    /// Assigned execution site.
    #[must_use]
    pub const fn site(&self) -> PlacementSite {
        self.site
    }

    /// Capability claim that produced this placement.
    #[must_use]
    pub const fn support(&self) -> &Support {
        &self.support
    }
}

/// Structured placement analysis for a complete logical plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlacementPlan {
    nodes: Box<[NodePlacement]>,
    transfer: Option<TransferBudget>,
}

impl PlacementPlan {
    /// Node decisions in logical-plan order.
    #[must_use]
    pub fn nodes(&self) -> &[NodePlacement] {
        &self.nodes
    }

    /// Hybrid transfer budget, if hybrid placement was requested.
    #[must_use]
    pub const fn transfer_budget(&self) -> Option<TransferBudget> {
        self.transfer
    }

    /// Whether every node is assigned to the engine.
    #[must_use]
    pub fn is_fully_engine(&self) -> bool {
        self.nodes
            .iter()
            .all(|node| node.site == PlacementSite::Engine)
    }
}

/// Analyzes exact semantic support and assigns every logical node.
pub fn analyze_placement(
    plan: &LogicalPlan,
    capabilities: &Capabilities,
    policy: PlacementPolicy,
) -> Result<PlacementPlan> {
    analyze_placement_with(plan, policy, |node| {
        capability_support_for_node(node, capabilities)
    })
}

/// Analyzes a plan using an engine's concrete, node-aware support refinement.
pub fn analyze_engine_placement(
    plan: &LogicalPlan,
    engine: &dyn Engine,
    policy: PlacementPolicy,
) -> Result<PlacementPlan> {
    analyze_placement_with(plan, policy, |node| engine.support_for_node(node))
}

fn analyze_placement_with(
    plan: &LogicalPlan,
    policy: PlacementPolicy,
    mut support_for: impl FnMut(&LogicalNode) -> Support,
) -> Result<PlacementPlan> {
    let transfer = match policy {
        PlacementPolicy::RemoteOnly => None,
        PlacementPolicy::Hybrid { transfer } => {
            transfer.validate()?;
            Some(transfer)
        }
    };

    let mut placements = Vec::with_capacity(plan.nodes().len());
    for (index, node) in plan.nodes().iter().enumerate() {
        let support = support_for(node);
        let upstream_local = has_local_dependency(node, &placements)?;
        let site = if support.is_supported() && !upstream_local {
            PlacementSite::Engine
        } else {
            match policy {
                PlacementPolicy::RemoteOnly => {
                    let reason = if upstream_local {
                        "an input already requires local residual execution"
                    } else {
                        support.reason().unwrap_or("no exact support")
                    };
                    return Err(Diagnostic::error(
                        "ENGINE-PLACEMENT-001",
                        format!("logical node {index} cannot execute remotely: {reason}"),
                    ));
                }
                PlacementPolicy::Hybrid { .. } => PlacementSite::LocalResidual,
            }
        };
        placements.push(NodePlacement {
            id: PlanId::from_index(index)?,
            site,
            support,
        });
    }

    Ok(PlacementPlan {
        nodes: placements.into_boxed_slice(),
        transfer,
    })
}

fn has_local_dependency(node: &LogicalNode, placements: &[NodePlacement]) -> Result<bool> {
    let is_local = |id: PlanId| -> Result<bool> {
        placements
            .get(id.index())
            .map(|placement| placement.site == PlacementSite::LocalResidual)
            .ok_or_else(|| {
                Diagnostic::error(
                    "ENGINE-PLACEMENT-004",
                    "logical node references an input outside the placement prefix",
                )
            })
    };
    let structural_local = match node {
        LogicalNode::Source { .. } => false,
        LogicalNode::Filter { input, .. }
        | LogicalNode::Project { input, .. }
        | LogicalNode::Aggregate { input, .. }
        | LogicalNode::Unnest { input, .. }
        | LogicalNode::Window { input, .. }
        | LogicalNode::Sort { input, .. }
        | LogicalNode::Distinct { input }
        | LogicalNode::Slice { input, .. } => is_local(*input)?,
        LogicalNode::Join { left, right, .. } | LogicalNode::Set { left, right, .. } => {
            is_local(*left)? || is_local(*right)?
        }
        _ => false,
    };
    if structural_local {
        return Ok(true);
    }
    for dependency in node_existential_dependencies(node) {
        if is_local(dependency)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn node_existential_dependencies(node: &LogicalNode) -> Vec<PlanId> {
    let mut dependencies = Vec::new();
    let mut append = |expression: &dol_core::plan::LogicalExpr| {
        dependencies.extend(expression.existential_dependencies());
    };
    match node {
        LogicalNode::Filter { condition, .. } => append(condition),
        LogicalNode::Project { projection, .. } => {
            for expression in projection.expressions() {
                append(expression);
            }
        }
        LogicalNode::Aggregate {
            groups, aggregates, ..
        } => {
            if let Some(groups) = groups {
                for expression in groups.expressions() {
                    append(expression);
                }
            }
            for aggregate in aggregates.aggregates() {
                if let Some(input) = aggregate.input() {
                    append(input);
                }
            }
        }
        LogicalNode::Window { window, .. } => {
            if let Some(input) = window.input() {
                append(input);
            }
            if let Some(partition) = window.partition() {
                for expression in partition.expressions() {
                    append(expression);
                }
            }
            for sort in window.order() {
                append(sort.expression());
            }
        }
        LogicalNode::Sort { keys, .. } => {
            for sort in keys {
                append(sort.expression());
            }
        }
        LogicalNode::Join {
            condition: Some(condition),
            ..
        } => append(condition),
        LogicalNode::Source { .. }
        | LogicalNode::Unnest { .. }
        | LogicalNode::Distinct { .. }
        | LogicalNode::Slice { .. }
        | LogicalNode::Join {
            condition: None, ..
        }
        | LogicalNode::Set { .. } => {}
        _ => {}
    }
    dependencies.sort_unstable_by_key(|id| id.index());
    dependencies.dedup();
    dependencies
}

/// Resolves one node from a coarse capability matrix.
#[doc(hidden)]
#[must_use]
pub fn capability_support_for_node(node: &LogicalNode, capabilities: &Capabilities) -> Support {
    let base = match node {
        LogicalNode::Source { .. } => capabilities.pipeline.source.clone(),
        LogicalNode::Filter { .. } => capabilities.pipeline.filter.clone(),
        LogicalNode::Project { .. } => capabilities.pipeline.project.clone(),
        LogicalNode::Aggregate { .. } => capabilities.pipeline.aggregate.clone(),
        LogicalNode::Unnest { .. } => capabilities.pipeline.unnest.clone(),
        LogicalNode::Window { .. } => capabilities.pipeline.window.clone(),
        LogicalNode::Sort { .. } => capabilities.pipeline.sort.clone(),
        LogicalNode::Distinct { .. } => capabilities.pipeline.distinct.clone(),
        LogicalNode::Slice { .. } => capabilities.pipeline.slice.clone(),
        LogicalNode::Join { .. } => capabilities.pipeline.join.clone(),
        LogicalNode::Set { .. } => capabilities.pipeline.set.clone(),
        _ => {
            return Support::unsupported(
                "logical node kind is not recognized by this engine SPI version",
            );
        }
    };
    if node_requires_existential_support(node) {
        combine_support(base, capabilities.pipeline.exists.clone())
    } else {
        base
    }
}

fn combine_support(left: Support, right: Support) -> Support {
    match (left, right) {
        (Support::Unsupported { reason }, _) | (_, Support::Unsupported { reason }) => {
            Support::Unsupported { reason }
        }
        (Support::ExactNative, Support::ExactNative) => Support::ExactNative,
        (Support::ExactNative | Support::ExactEmulated, Support::ExactEmulated)
        | (Support::ExactEmulated, Support::ExactNative) => Support::ExactEmulated,
    }
}

fn node_requires_existential_support(node: &LogicalNode) -> bool {
    let expression_requires = |expression: &dol_core::plan::LogicalExpr| {
        expression.outer_scope_count() != 0 || !expression.existential_dependencies().is_empty()
    };
    match node {
        LogicalNode::Filter { condition, .. } => expression_requires(condition),
        LogicalNode::Project { projection, .. } => {
            projection.expressions().iter().any(expression_requires)
        }
        LogicalNode::Aggregate {
            groups, aggregates, ..
        } => {
            groups
                .as_deref()
                .is_some_and(|projection| projection.expressions().iter().any(expression_requires))
                || aggregates
                    .aggregates()
                    .iter()
                    .filter_map(dol_core::plan::LogicalAggregate::input)
                    .any(expression_requires)
        }
        LogicalNode::Window { window, .. } => {
            window.input().is_some_and(expression_requires)
                || window.partition().is_some_and(|projection| {
                    projection.expressions().iter().any(expression_requires)
                })
                || window
                    .order()
                    .iter()
                    .any(|sort| expression_requires(sort.expression()))
        }
        LogicalNode::Sort { keys, .. } => keys
            .iter()
            .any(|sort| expression_requires(sort.expression())),
        LogicalNode::Join { condition, .. } => {
            condition.as_deref().is_some_and(expression_requires)
        }
        LogicalNode::Source { .. }
        | LogicalNode::Unnest { .. }
        | LogicalNode::Distinct { .. }
        | LogicalNode::Slice { .. }
        | LogicalNode::Set { .. } => false,
        _ => false,
    }
}
