//! Conservative semantics-preserving logical-plan rewrites.

use crate::diagnostic::{Diagnostic, Result};

use super::{LogicalNode, LogicalPlan, PlanId};

pub(super) fn optimize(plan: &LogicalPlan) -> Result<LogicalPlan> {
    let mut nodes = Vec::with_capacity(plan.nodes.len());
    let mut remap = Vec::with_capacity(plan.nodes.len());

    for node in &plan.nodes {
        let node = remap_node(node, &remap)?;
        if let Some(existing) = redundant_target(&node, &nodes) {
            remap.push(existing);
            continue;
        }

        let id = PlanId::from_index(nodes.len())?;
        nodes.push(node);
        remap.push(id);
    }

    let root = mapped(plan.root, &remap)?;
    Ok(LogicalPlan::from_optimized(
        nodes,
        root,
        plan.output.clone(),
        plan.fingerprint,
    ))
}

fn redundant_target(node: &LogicalNode, nodes: &[LogicalNode]) -> Option<PlanId> {
    match node {
        LogicalNode::Slice {
            input,
            offset: 0,
            limit: None,
        } => Some(*input),
        LogicalNode::Distinct { input }
            if matches!(nodes.get(input.index()), Some(LogicalNode::Distinct { .. })) =>
        {
            Some(*input)
        }
        _ => None,
    }
}

fn remap_node(node: &LogicalNode, remap: &[PlanId]) -> Result<LogicalNode> {
    Ok(match node {
        LogicalNode::Source { model, alias } => LogicalNode::Source {
            model,
            alias: alias.clone(),
        },
        LogicalNode::Filter { input, condition } => {
            let mut condition = condition.clone();
            condition.remap_plan_ids(remap)?;
            LogicalNode::Filter {
                input: mapped(*input, remap)?,
                condition,
            }
        }
        LogicalNode::Project { input, projection } => LogicalNode::Project {
            input: mapped(*input, remap)?,
            projection: projection.clone(),
        },
        LogicalNode::Aggregate {
            input,
            groups,
            aggregates,
        } => LogicalNode::Aggregate {
            input: mapped(*input, remap)?,
            groups: groups.clone(),
            aggregates: aggregates.clone(),
        },
        LogicalNode::Unnest { input, element } => LogicalNode::Unnest {
            input: mapped(*input, remap)?,
            element: element.clone(),
        },
        LogicalNode::Window { input, window } => LogicalNode::Window {
            input: mapped(*input, remap)?,
            window: window.clone(),
        },
        LogicalNode::Sort { input, keys } => LogicalNode::Sort {
            input: mapped(*input, remap)?,
            keys: keys.clone(),
        },
        LogicalNode::Distinct { input } => LogicalNode::Distinct {
            input: mapped(*input, remap)?,
        },
        LogicalNode::Slice {
            input,
            offset,
            limit,
        } => LogicalNode::Slice {
            input: mapped(*input, remap)?,
            offset: *offset,
            limit: *limit,
        },
        LogicalNode::Join {
            left,
            right,
            kind,
            condition,
        } => LogicalNode::Join {
            left: mapped(*left, remap)?,
            right: mapped(*right, remap)?,
            kind: *kind,
            condition: condition.clone(),
        },
        LogicalNode::Set {
            left,
            right,
            operator,
        } => LogicalNode::Set {
            left: mapped(*left, remap)?,
            right: mapped(*right, remap)?,
            operator: *operator,
        },
    })
}

fn mapped(id: PlanId, remap: &[PlanId]) -> Result<PlanId> {
    remap.get(id.index()).copied().ok_or_else(|| {
        Diagnostic::error(
            "PIPELINE-OPT-001",
            "optimizer encountered a non-topological logical-plan reference",
        )
    })
}
