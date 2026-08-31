//! Structured explain-plan vocabulary derived from logical placement.

use dol_core::fingerprint::Fingerprint;
use dol_core::plan::{LogicalNode, LogicalPlan, PlanId};

use crate::execute::EngineInfo;
use crate::placement::{PlacementPlan, PlacementSite};

/// One explain step corresponding to a logical-plan node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExplainStep {
    node: PlanId,
    operation: &'static str,
    site: PlacementSite,
    detail: String,
}

impl ExplainStep {
    /// Logical plan node identifier.
    #[must_use]
    pub const fn node(&self) -> PlanId {
        self.node
    }

    /// Stable logical operation label.
    #[must_use]
    pub const fn operation(&self) -> &'static str {
        self.operation
    }

    /// Assigned execution site.
    #[must_use]
    pub const fn site(&self) -> PlacementSite {
        self.site
    }

    /// Exact-support explanation.
    #[must_use]
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

/// Structured planner explanation independent of backend SQL/BSON syntax.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExplainPlan {
    engine_kind: &'static str,
    engine_version: &'static str,
    fingerprint: Fingerprint,
    steps: Box<[ExplainStep]>,
}

impl ExplainPlan {
    /// Builds an explain model from an already validated placement decision.
    #[must_use]
    pub fn new(engine: &EngineInfo, plan: &LogicalPlan, placement: &PlacementPlan) -> Self {
        let steps = plan
            .nodes()
            .iter()
            .zip(placement.nodes())
            .map(|(node, assigned)| ExplainStep {
                node: assigned.id(),
                operation: operation_name(node),
                site: assigned.site(),
                detail: match (assigned.site(), assigned.support()) {
                    (PlacementSite::LocalResidual, support) if support.is_supported() => {
                        "local residual because an upstream input already requires local execution"
                            .to_owned()
                    }
                    (_, crate::Support::ExactNative) => "exact native".to_owned(),
                    (_, crate::Support::ExactEmulated) => "exact emulated".to_owned(),
                    (_, crate::Support::Unsupported { reason }) => reason.clone(),
                },
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Self {
            engine_kind: engine.kind,
            engine_version: engine.version,
            fingerprint: plan.fingerprint(),
            steps,
        }
    }

    /// Stable engine kind.
    #[must_use]
    pub const fn engine_kind(&self) -> &'static str {
        self.engine_kind
    }

    /// Adapter implementation version.
    #[must_use]
    pub const fn engine_version(&self) -> &'static str {
        self.engine_version
    }

    /// Canonical logical-plan fingerprint.
    #[must_use]
    pub const fn fingerprint(&self) -> Fingerprint {
        self.fingerprint
    }

    /// Explain steps in logical-plan order.
    #[must_use]
    pub fn steps(&self) -> &[ExplainStep] {
        &self.steps
    }
}

fn operation_name(node: &LogicalNode) -> &'static str {
    match node {
        LogicalNode::Source { .. } => "source",
        LogicalNode::Filter { .. } => "filter",
        LogicalNode::Project { .. } => "project",
        LogicalNode::Aggregate { .. } => "aggregate",
        LogicalNode::Unnest { .. } => "unnest",
        LogicalNode::Window { .. } => "window",
        LogicalNode::Sort { .. } => "sort",
        LogicalNode::Distinct { .. } => "distinct",
        LogicalNode::Slice { .. } => "slice",
        LogicalNode::Join { .. } => "join",
        LogicalNode::Set { .. } => "set",
        _ => "unknown",
    }
}
