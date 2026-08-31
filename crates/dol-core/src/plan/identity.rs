//! Canonical semantic fingerprints for logical-plan stages.

use crate::analytics::{AggregateKind, AggregateSelectionKind};
use crate::fingerprint::{CanonicalHasher, Fingerprint, hash_type_def};
use crate::model::ModelDef;
use crate::types::TypeDef;

use super::{
    JoinKind, LogicalAggregateSelection, LogicalExpr, LogicalProjection, LogicalWindow, PlanOutput,
    ProjectionKind, SetOperator, SortDirection, SortExpr, WindowFrameBound, WindowFunctionKind,
};

pub(crate) fn fingerprint_source(model: &ModelDef) -> Fingerprint {
    let mut hasher = CanonicalHasher::new(b"pipeline/source/v1");
    hasher.str(model.key().as_str());
    hasher.bytes(model.fingerprint().as_bytes());
    hasher.finish()
}

pub(crate) fn fingerprint_unary_stage(domain: &'static [u8], input: Fingerprint) -> Fingerprint {
    let mut hasher = CanonicalHasher::new(domain);
    hasher.bytes(input.as_bytes());
    hasher.finish()
}

pub(crate) fn fingerprint_expression_stage(
    domain: &'static [u8],
    input: Fingerprint,
    expression: &LogicalExpr,
) -> Fingerprint {
    let mut hasher = CanonicalHasher::new(domain);
    hasher.bytes(input.as_bytes());
    hasher.bytes(expression.bound_fingerprint().as_bytes());
    hasher.finish()
}

pub(crate) fn fingerprint_projection_stage(
    input: Fingerprint,
    projection: &LogicalProjection,
) -> Fingerprint {
    let mut hasher = CanonicalHasher::new(b"pipeline/project/v3");
    hasher.bytes(input.as_bytes());
    hash_projection(&mut hasher, projection);
    hasher.finish()
}

pub(crate) fn fingerprint_aggregate_stage(
    input: Fingerprint,
    groups: Option<&LogicalProjection>,
    selection: &LogicalAggregateSelection,
) -> Fingerprint {
    let mut hasher = CanonicalHasher::new(b"pipeline/aggregate/v1");
    hasher.bytes(input.as_bytes());
    match groups {
        Some(groups) => {
            hasher.u8(1);
            hash_projection(&mut hasher, groups);
        }
        None => hasher.u8(0),
    }
    hasher.u8(match selection.kind() {
        AggregateSelectionKind::Value => 0,
        AggregateSelectionKind::Tuple => 1,
    });
    hasher.u64(selection.aggregates().len() as u64);
    for aggregate in selection.aggregates() {
        hasher.u8(match aggregate.kind() {
            AggregateKind::CountRows => 0,
            AggregateKind::CountPresent => 1,
            AggregateKind::Sum => 2,
            AggregateKind::Min => 3,
            AggregateKind::Max => 4,
        });
        match aggregate.input() {
            Some(expression) => {
                hasher.u8(1);
                hasher.bytes(expression.bound_fingerprint().as_bytes());
            }
            None => hasher.u8(0),
        }
        hash_type_def(&mut hasher, aggregate.type_def());
    }
    hash_type_def(&mut hasher, selection.type_def());
    hasher.finish()
}

fn hash_projection(hasher: &mut CanonicalHasher, projection: &LogicalProjection) {
    hasher.u8(match projection.kind() {
        ProjectionKind::Value => 0,
        ProjectionKind::Tuple => 1,
        ProjectionKind::Record => 2,
    });
    hasher.u64(projection.expressions().len() as u64);
    for (index, expression) in projection.expressions().iter().enumerate() {
        if projection.kind() == ProjectionKind::Record
            && let Some(name) = projection.names().get(index)
        {
            hasher.str(name);
        }
        hasher.bytes(expression.bound_fingerprint().as_bytes());
    }
    hash_type_def(hasher, projection.type_def());
}

pub(crate) fn fingerprint_sort_stage(input: Fingerprint, keys: &[SortExpr]) -> Fingerprint {
    let mut hasher = CanonicalHasher::new(b"pipeline/sort/v2");
    hasher.bytes(input.as_bytes());
    hasher.u64(keys.len() as u64);
    for key in keys {
        hasher.u8(match key.direction() {
            SortDirection::Ascending => 0,
            SortDirection::Descending => 1,
        });
        hasher.bytes(key.expression().bound_fingerprint().as_bytes());
    }
    hasher.finish()
}

pub(crate) fn fingerprint_unnest_stage(input: Fingerprint, element: &TypeDef) -> Fingerprint {
    let mut hasher = CanonicalHasher::new(b"pipeline/unnest/v1");
    hasher.bytes(input.as_bytes());
    hash_type_def(&mut hasher, element);
    hasher.finish()
}

pub(crate) fn fingerprint_window_stage(input: Fingerprint, window: &LogicalWindow) -> Fingerprint {
    let mut hasher = CanonicalHasher::new(b"pipeline/window/v1");
    hasher.bytes(input.as_bytes());
    hasher.u8(match window.kind() {
        WindowFunctionKind::RowNumber => 0,
        WindowFunctionKind::Rank => 1,
        WindowFunctionKind::DenseRank => 2,
        WindowFunctionKind::Sum => 3,
    });
    match window.input() {
        Some(expression) => {
            hasher.u8(1);
            hasher.bytes(expression.bound_fingerprint().as_bytes());
        }
        None => hasher.u8(0),
    }
    match window.partition() {
        Some(partition) => {
            hasher.u8(1);
            hash_projection(&mut hasher, partition);
        }
        None => hasher.u8(0),
    }
    hasher.u64(window.order().len() as u64);
    for key in window.order() {
        hasher.u8(match key.direction() {
            SortDirection::Ascending => 0,
            SortDirection::Descending => 1,
        });
        hasher.bytes(key.expression().bound_fingerprint().as_bytes());
    }
    match window.frame() {
        Some(frame) => {
            hasher.u8(1);
            hash_window_bound(&mut hasher, frame.start());
            hash_window_bound(&mut hasher, frame.end());
        }
        None => hasher.u8(0),
    }
    hash_type_def(&mut hasher, window.type_def());
    hasher.finish()
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

pub(crate) fn fingerprint_slice_stage(
    input: Fingerprint,
    offset: u64,
    limit: Option<u64>,
) -> Fingerprint {
    let mut hasher = CanonicalHasher::new(b"pipeline/slice/v1");
    hasher.bytes(input.as_bytes());
    hasher.u64(offset);
    match limit {
        Some(limit) => {
            hasher.u8(1);
            hasher.u64(limit);
        }
        None => hasher.u8(0),
    }
    hasher.finish()
}

pub(crate) fn fingerprint_join_stage(
    left: Fingerprint,
    right: Fingerprint,
    kind: JoinKind,
    condition: Option<&LogicalExpr>,
) -> Fingerprint {
    let mut hasher = CanonicalHasher::new(b"pipeline/join/v1");
    hasher.bytes(left.as_bytes());
    hasher.bytes(right.as_bytes());
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
            hasher.bytes(condition.bound_fingerprint().as_bytes());
        }
        None => hasher.u8(0),
    }
    hasher.finish()
}

pub(crate) fn fingerprint_set_stage(
    left: Fingerprint,
    right: Fingerprint,
    operator: SetOperator,
) -> Fingerprint {
    let (left, right) =
        if matches!(operator, SetOperator::Except) || left.as_bytes() <= right.as_bytes() {
            (left, right)
        } else {
            (right, left)
        };
    let mut hasher = CanonicalHasher::new(b"pipeline/set/v1");
    hasher.bytes(left.as_bytes());
    hasher.bytes(right.as_bytes());
    hasher.u8(match operator {
        SetOperator::Union => 0,
        SetOperator::UnionAll => 1,
        SetOperator::Intersect => 2,
        SetOperator::Except => 3,
    });
    hasher.finish()
}

pub(super) fn fingerprint_plan(root: Fingerprint, output: &PlanOutput) -> Fingerprint {
    let mut hasher = CanonicalHasher::new(b"pipeline/plan/v2");
    hasher.bytes(root.as_bytes());
    hash_plan_output(&mut hasher, output);
    hasher.finish()
}

fn hash_plan_output(hasher: &mut CanonicalHasher, output: &PlanOutput) {
    match output {
        PlanOutput::Model(model) => {
            hasher.u8(0);
            hasher.str(model.key().as_str());
            hasher.bytes(model.fingerprint().as_bytes());
        }
        PlanOutput::Value(ty) => {
            hasher.u8(1);
            hash_type_def(hasher, ty);
        }
        PlanOutput::Product(outputs) => {
            hasher.u8(2);
            hasher.u64(outputs.len() as u64);
            for output in outputs {
                hash_plan_output(hasher, output);
            }
        }
        PlanOutput::Nullable(output) => {
            hasher.u8(3);
            hash_plan_output(hasher, output);
        }
    }
}
