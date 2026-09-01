//! Binding-independent canonical identity for authoring pipelines.
//!
//! Existential expressions need to fingerprint nested pipelines before an
//! enclosing scope is available. This module hashes symbolic pipeline structure
//! without attempting expression binding; validated logical plans retain their
//! separate bound fingerprints.

use std::collections::HashMap;
use std::sync::Arc;

use crate::analytics::{AggregateKind, AggregateSelectionKind};
use crate::diagnostic::{Diagnostic, Result};
use crate::expr::fingerprint_expression_spec;
use crate::fingerprint::{CanonicalHasher, Fingerprint, hash_type_def};
use crate::limits::PipelineLimits;
use crate::plan::{
    JoinKind, ProjectionKind, SetOperator, SortDirection, WindowFrameBound, WindowFunctionKind,
};

use super::{PipelineKind, PipelineNode, projection::ProjectionSpec, window::WindowSpec};

pub(super) fn fingerprint(root: &Arc<PipelineNode>) -> Result<Fingerprint> {
    let limits = PipelineLimits::default();
    let nodes = collect_structural_nodes(root, limits.max_nodes)?;
    let mut fingerprints = HashMap::with_capacity(nodes.len());

    for node in nodes {
        let value = fingerprint_node(&node, &fingerprints)?;
        fingerprints.insert(Arc::as_ptr(&node), value);
    }

    fingerprints
        .get(&Arc::as_ptr(root))
        .copied()
        .ok_or_else(|| {
            Diagnostic::error(
                "PIPELINE-FINGERPRINT-001",
                "authoring pipeline contains no semantic source",
            )
        })
}

fn collect_structural_nodes(
    root: &Arc<PipelineNode>,
    max_nodes: usize,
) -> Result<Vec<Arc<PipelineNode>>> {
    let mut order = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut stack = vec![(Arc::clone(root), false)];

    while let Some((node, expanded)) = stack.pop() {
        let pointer = Arc::as_ptr(&node);
        if expanded {
            order.push(node);
            continue;
        }
        if !seen.insert(pointer) {
            continue;
        }
        if seen.len() > max_nodes {
            return Err(Diagnostic::error(
                "PIPELINE-LIMIT-002",
                format!("pipeline has more than {max_nodes} logical nodes"),
            ));
        }

        stack.push((Arc::clone(&node), true));
        let mut inputs = Vec::with_capacity(2);
        node.push_structural_inputs(&mut inputs);
        for input in inputs {
            stack.push((input, false));
        }
    }

    Ok(order)
}

fn fingerprint_node(
    node: &Arc<PipelineNode>,
    fingerprints: &HashMap<*const PipelineNode, Fingerprint>,
) -> Result<Fingerprint> {
    let mut hasher = CanonicalHasher::new(b"pipeline/authoring/v1");
    match node.kind() {
        PipelineKind::Source(source) => {
            hasher.u8(0);
            let model = source.resolve()?;
            hasher.str(model.key().as_str());
            hasher.bytes(model.fingerprint().as_bytes());
        }
        PipelineKind::Filter { input, condition } => {
            hasher.u8(1);
            hash_input(&mut hasher, fingerprints, input)?;
            hash_expression(&mut hasher, condition)?;
        }
        PipelineKind::Project { input, projection } => {
            hasher.u8(2);
            hash_input(&mut hasher, fingerprints, input)?;
            hash_projection(&mut hasher, projection)?;
        }
        PipelineKind::Aggregate {
            input,
            groups,
            aggregates,
        } => {
            hasher.u8(3);
            hash_input(&mut hasher, fingerprints, input)?;
            match groups {
                Some(groups) => {
                    hasher.u8(1);
                    hash_projection(&mut hasher, groups)?;
                }
                None => hasher.u8(0),
            }
            hasher.u8(match aggregates.kind {
                AggregateSelectionKind::Value => 0,
                AggregateSelectionKind::Tuple => 1,
            });
            hasher.u64(aggregates.aggregates.len() as u64);
            for aggregate in &aggregates.aggregates {
                hasher.u8(match aggregate.kind {
                    AggregateKind::CountRows => 0,
                    AggregateKind::CountPresent => 1,
                    AggregateKind::Sum => 2,
                    AggregateKind::Min => 3,
                    AggregateKind::Max => 4,
                });
                match &aggregate.input {
                    Some(input) => {
                        hasher.u8(1);
                        hash_expression(&mut hasher, input)?;
                    }
                    None => hasher.u8(0),
                }
                hash_type_def(&mut hasher, &aggregate.ty);
            }
            hash_type_def(&mut hasher, &aggregates.ty);
        }
        PipelineKind::Unnest { input } => {
            hasher.u8(4);
            hash_input(&mut hasher, fingerprints, input)?;
        }
        PipelineKind::Window { input, window } => {
            hasher.u8(5);
            hash_input(&mut hasher, fingerprints, input)?;
            hash_window(&mut hasher, window)?;
        }
        PipelineKind::Sort {
            input,
            expression,
            direction,
        } => {
            hasher.u8(6);
            hash_input(&mut hasher, fingerprints, input)?;
            hasher.u8(match direction {
                SortDirection::Ascending => 0,
                SortDirection::Descending => 1,
            });
            hash_expression(&mut hasher, expression)?;
        }
        PipelineKind::Distinct { input } => {
            hasher.u8(7);
            hash_input(&mut hasher, fingerprints, input)?;
        }
        PipelineKind::Slice {
            input,
            offset,
            limit,
        } => {
            hasher.u8(8);
            hash_input(&mut hasher, fingerprints, input)?;
            hasher.u64(*offset);
            match limit {
                Some(limit) => {
                    hasher.u8(1);
                    hasher.u64(*limit);
                }
                None => hasher.u8(0),
            }
        }
        PipelineKind::Join {
            left,
            right,
            kind,
            condition,
        } => {
            hasher.u8(9);
            hash_input(&mut hasher, fingerprints, left)?;
            hash_input(&mut hasher, fingerprints, right)?;
            hasher.u8(match kind {
                JoinKind::Inner => 0,
                JoinKind::Cross => 1,
                JoinKind::Left => 2,
                JoinKind::Right => 3,
                JoinKind::Full => 4,
            });
            match condition {
                Some(condition) => {
                    hasher.u8(1);
                    hash_expression(&mut hasher, condition)?;
                }
                None => hasher.u8(0),
            }
        }
        PipelineKind::Set {
            left,
            right,
            operator,
        } => {
            hasher.u8(10);
            let left = require_input(fingerprints, left)?;
            let right = require_input(fingerprints, right)?;
            let (left, right) =
                if matches!(operator, SetOperator::Except) || left.as_bytes() <= right.as_bytes() {
                    (left, right)
                } else {
                    (right, left)
                };
            hasher.bytes(left.as_bytes());
            hasher.bytes(right.as_bytes());
            hasher.u8(match operator {
                SetOperator::Union => 0,
                SetOperator::UnionAll => 1,
                SetOperator::Intersect => 2,
                SetOperator::Except => 3,
            });
        }
    }
    Ok(hasher.finish())
}

fn hash_input(
    hasher: &mut CanonicalHasher,
    fingerprints: &HashMap<*const PipelineNode, Fingerprint>,
    input: &Arc<PipelineNode>,
) -> Result<()> {
    hasher.bytes(require_input(fingerprints, input)?.as_bytes());
    Ok(())
}

fn require_input(
    fingerprints: &HashMap<*const PipelineNode, Fingerprint>,
    input: &Arc<PipelineNode>,
) -> Result<Fingerprint> {
    fingerprints
        .get(&Arc::as_ptr(input))
        .copied()
        .ok_or_else(|| {
            Diagnostic::error(
                "PIPELINE-FINGERPRINT-002",
                "authoring fingerprint encountered a non-topological pipeline input",
            )
        })
}

fn hash_expression(
    hasher: &mut CanonicalHasher,
    expression: &crate::expr::ExpressionSpec,
) -> Result<()> {
    hasher.bytes(fingerprint_expression_spec(expression)?.as_bytes());
    Ok(())
}

fn hash_projection(hasher: &mut CanonicalHasher, projection: &ProjectionSpec) -> Result<()> {
    hasher.u8(match projection.kind {
        ProjectionKind::Value => 0,
        ProjectionKind::Tuple => 1,
        ProjectionKind::Record => 2,
    });

    if projection.kind == ProjectionKind::Record {
        let mut fields = projection
            .names
            .iter()
            .zip(projection.expressions.iter())
            .map(|(name, expression)| Ok((name.as_ref(), fingerprint_expression_spec(expression)?)))
            .collect::<Result<Vec<_>>>()?;
        fields.sort_by(|left, right| left.0.cmp(right.0));
        hasher.u64(fields.len() as u64);
        for (name, fingerprint) in fields {
            hasher.str(name);
            hasher.bytes(fingerprint.as_bytes());
        }
    } else {
        hasher.u64(projection.expressions.len() as u64);
        for expression in &projection.expressions {
            hash_expression(hasher, expression)?;
        }
    }
    hash_type_def(hasher, &projection.ty);
    Ok(())
}

fn hash_window(hasher: &mut CanonicalHasher, window: &WindowSpec) -> Result<()> {
    hasher.u8(match window.kind {
        WindowFunctionKind::RowNumber => 0,
        WindowFunctionKind::Rank => 1,
        WindowFunctionKind::DenseRank => 2,
        WindowFunctionKind::Sum => 3,
    });
    match &window.input {
        Some(input) => {
            hasher.u8(1);
            hash_expression(hasher, input)?;
        }
        None => hasher.u8(0),
    }
    match &window.partition {
        Some(partition) => {
            hasher.u8(1);
            hash_projection(hasher, partition)?;
        }
        None => hasher.u8(0),
    }
    hasher.u64(window.order.len() as u64);
    for key in &window.order {
        hasher.u8(match key.direction {
            SortDirection::Ascending => 0,
            SortDirection::Descending => 1,
        });
        hash_expression(hasher, &key.expression)?;
    }
    match window.frame {
        Some(frame) => {
            hasher.u8(1);
            hash_window_bound(hasher, frame.start());
            hash_window_bound(hasher, frame.end());
        }
        None => hasher.u8(0),
    }
    hash_type_def(hasher, &window.ty);
    Ok(())
}

fn hash_window_bound(hasher: &mut CanonicalHasher, bound: WindowFrameBound) {
    match bound {
        WindowFrameBound::UnboundedPreceding => hasher.u8(0),
        WindowFrameBound::Preceding(distance) => {
            hasher.u8(1);
            hasher.u64(distance);
        }
        WindowFrameBound::CurrentRow => hasher.u8(2),
        WindowFrameBound::Following(distance) => {
            hasher.u8(3);
            hasher.u64(distance);
        }
        WindowFrameBound::UnboundedFollowing => hasher.u8(4),
    }
}
