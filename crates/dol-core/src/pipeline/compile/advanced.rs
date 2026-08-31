//! Slice-4 compilation for existential subqueries, unnesting, and windows.

use std::collections::HashMap;
use std::sync::Arc;

use crate::diagnostic::{Diagnostic, Result};
use crate::expr::prepare_expression;
use crate::limits::PipelineLimits;
use crate::plan::{
    LogicalExpr, LogicalNode, LogicalWindow, LogicalWindowSpec, PlanBuilder, RowsFrame, SortExpr,
    WindowFrameBound, WindowFunctionKind, fingerprint_unnest_stage, fingerprint_window_stage,
};
use crate::types::{DataType, TypeShape};

use super::{Compiled, Output, Scope, bind_context, compile_projection, require_compiled};
use crate::pipeline::PipelineNode;
use crate::pipeline::window::{WindowInputMode, WindowSpec};

pub(super) fn compile_unnest(
    compiled: &HashMap<*const PipelineNode, Compiled>,
    builder: &mut PlanBuilder,
    input_node: &Arc<PipelineNode>,
) -> Result<Compiled> {
    let input = require_compiled(compiled, input_node)?;
    let Output::Value(input_ty) = &input.output else {
        return Err(Diagnostic::error(
            "PIPELINE-UNNEST-001",
            "unnest requires a list-valued pipeline output",
        ));
    };
    let TypeShape::List(element) = input_ty.shape() else {
        return Err(Diagnostic::error(
            "PIPELINE-UNNEST-001",
            format!(
                "unnest requires a list output; type `{}` is not a list",
                input_ty.key().as_str()
            ),
        ));
    };
    let element = element.as_ref().clone();
    let fingerprint = fingerprint_unnest_stage(builder.fingerprint(input.id)?, &element);
    let id = builder.push(
        LogicalNode::Unnest {
            input: input.id,
            element: element.clone(),
        },
        fingerprint,
    )?;

    Ok(Compiled {
        id,
        scopes: Vec::new(),
        output: Output::Value(element),
    })
}

pub(super) fn compile_window(
    compiled: &HashMap<*const PipelineNode, Compiled>,
    builder: &mut PlanBuilder,
    limits: PipelineLimits,
    input_node: &Arc<PipelineNode>,
    spec: &WindowSpec,
) -> Result<Compiled> {
    let input = require_compiled(compiled, input_node)?;
    if input.scopes.is_empty() {
        return Err(Diagnostic::error(
            "PIPELINE-WINDOW-001",
            "window expressions require at least one visible model source scope",
        ));
    }
    let context = bind_context(&input.scopes);
    let partition = spec
        .partition
        .as_ref()
        .map(|projection| compile_projection(projection, &context, limits))
        .transpose()?;
    if let Some(partition) = &partition {
        let properties = partition.type_def().properties();
        if !properties.equality || !properties.keyable {
            return Err(Diagnostic::error(
                "PIPELINE-WINDOW-002",
                format!(
                    "window partition type `{}` requires stable equality and key semantics",
                    partition.type_def().key().as_str()
                ),
            ));
        }
    }

    let mut order = Vec::with_capacity(spec.order.len());
    for key in &spec.order {
        let expression = LogicalExpr::from_prepared(prepare_expression(
            &key.expression,
            &context,
            limits.expression,
        )?);
        let properties = expression.type_def().properties();
        if !properties.ordering || !properties.equality || !properties.keyable {
            return Err(Diagnostic::error(
                "PIPELINE-WINDOW-003",
                format!(
                    "window ordering type `{}` requires stable ordering, equality, and key semantics",
                    expression.type_def().key().as_str()
                ),
            ));
        }
        order.push(SortExpr::new(expression, key.direction));
    }

    let value_input = spec
        .input
        .as_ref()
        .map(|expression| {
            prepare_expression(expression, &context, limits.expression)
                .map(LogicalExpr::from_prepared)
        })
        .transpose()?;

    validate_window(spec, value_input.as_ref(), &order, &input.scopes)?;

    let window = LogicalWindow::new(LogicalWindowSpec {
        kind: spec.kind,
        input: value_input,
        partition: partition.map(Box::new),
        order: order.into_boxed_slice(),
        frame: spec.frame,
        ty: spec.ty.clone(),
    });
    let fingerprint = fingerprint_window_stage(builder.fingerprint(input.id)?, &window);
    let id = builder.push(
        LogicalNode::Window {
            input: input.id,
            window: Box::new(window),
        },
        fingerprint,
    )?;
    let output = Output::Product(
        vec![input.output.clone(), Output::Value(spec.ty.clone())].into_boxed_slice(),
    );

    Ok(Compiled {
        id,
        scopes: input.scopes,
        output,
    })
}

fn validate_window(
    spec: &WindowSpec,
    input: Option<&LogicalExpr>,
    order: &[SortExpr],
    scopes: &[Scope],
) -> Result<()> {
    match spec.kind {
        WindowFunctionKind::RowNumber => {
            validate_ranking_shape(spec, input, order, "row_number")?;
            require_total_model_order(scopes, order, "row_number")?;
        }
        WindowFunctionKind::Rank => validate_ranking_shape(spec, input, order, "rank")?,
        WindowFunctionKind::DenseRank => {
            validate_ranking_shape(spec, input, order, "dense_rank")?;
        }
        WindowFunctionKind::Sum => validate_window_sum(spec, input, order, scopes)?,
    }
    Ok(())
}

fn validate_ranking_shape(
    spec: &WindowSpec,
    input: Option<&LogicalExpr>,
    order: &[SortExpr],
    operation: &str,
) -> Result<()> {
    if input.is_some() || spec.ty != u64::type_def() {
        return Err(Diagnostic::error(
            "PIPELINE-WINDOW-004",
            format!("{operation} takes no value input and must produce dol/u64"),
        ));
    }
    if order.is_empty() {
        return Err(Diagnostic::error(
            "PIPELINE-WINDOW-005",
            format!("{operation} requires at least one explicit ordering key"),
        ));
    }
    if spec.frame.is_some() {
        return Err(Diagnostic::error(
            "PIPELINE-WINDOW-006",
            format!("{operation} does not accept a rows frame"),
        ));
    }
    Ok(())
}

fn validate_window_sum(
    spec: &WindowSpec,
    input: Option<&LogicalExpr>,
    order: &[SortExpr],
    scopes: &[Scope],
) -> Result<()> {
    let input = input.ok_or_else(|| {
        Diagnostic::error(
            "PIPELINE-WINDOW-007",
            "window_sum requires one input expression",
        )
    })?;
    if spec.input_mode == WindowInputMode::NonNull && input.type_def().is_nullable() {
        return Err(Diagnostic::error(
            "PIPELINE-WINDOW-008",
            "window_sum received a nullable source; use window_sum_present to state null/missing handling explicitly",
        ));
    }
    if !input.type_def().properties().exact_numeric {
        return Err(Diagnostic::error(
            "PIPELINE-WINDOW-009",
            format!(
                "window_sum requires exact numeric semantics; type `{}` is not exact numeric",
                input.type_def().key().as_str()
            ),
        ));
    }
    if spec.ty != input.type_def().clone().nullable() {
        return Err(Diagnostic::error(
            "PIPELINE-WINDOW-010",
            "window_sum output type does not match the nullable input semantic type",
        ));
    }
    let frame = spec.frame.ok_or_else(|| {
        Diagnostic::error(
            "PIPELINE-WINDOW-011",
            "window_sum requires an explicit rows frame; use RowsFrame::all() or another explicit frame",
        )
    })?;
    validate_frame(frame)?;
    if frame != RowsFrame::all() {
        if order.is_empty() {
            return Err(Diagnostic::error(
                "PIPELINE-WINDOW-012",
                "bounded/current-row window_sum frames require an explicit total ordering",
            ));
        }
        require_total_model_order(scopes, order, "window_sum rows frame")?;
    }
    Ok(())
}

fn validate_frame(frame: RowsFrame) -> Result<()> {
    if matches!(frame.start(), WindowFrameBound::UnboundedFollowing)
        || matches!(frame.end(), WindowFrameBound::UnboundedPreceding)
        || bound_position(frame.start()) > bound_position(frame.end())
    {
        return Err(Diagnostic::error(
            "PIPELINE-WINDOW-013",
            "window rows frame start must not follow its end",
        ));
    }
    Ok(())
}

fn bound_position(bound: WindowFrameBound) -> i128 {
    match bound {
        WindowFrameBound::UnboundedPreceding => i128::MIN,
        WindowFrameBound::Preceding(distance) => -(i128::from(distance)),
        WindowFrameBound::CurrentRow => 0,
        WindowFrameBound::Following(distance) => i128::from(distance),
        WindowFrameBound::UnboundedFollowing => i128::MAX,
    }
}

fn require_total_model_order(scopes: &[Scope], order: &[SortExpr], operation: &str) -> Result<()> {
    let [scope] = scopes else {
        return Err(Diagnostic::error(
            "PIPELINE-WINDOW-014",
            format!(
                "{operation} currently requires exactly one model source scope so total ordering can be proven"
            ),
        ));
    };
    let identity = scope.model.identity().ok_or_else(|| {
        Diagnostic::error(
            "PIPELINE-WINDOW-015",
            format!(
                "{operation} requires a model identity in its ordering to make peer order deterministic"
            ),
        )
    })?;
    if !scope.identity_unique {
        return Err(Diagnostic::error(
            "PIPELINE-WINDOW-018",
            format!("{operation} requires source identity to remain unique in the current rowset"),
        ));
    }

    for key in identity.fields() {
        let slot = scope.model.field(key.as_str()).ok_or_else(|| {
            Diagnostic::error(
                "PIPELINE-WINDOW-016",
                "model identity references an unavailable canonical field",
            )
        })?;
        let present = order
            .iter()
            .filter_map(|sort| sort.expression().direct_field())
            .any(|(scope_index, slot_index)| scope_index == 0 && slot_index == slot.slot().index());
        if !present {
            return Err(Diagnostic::error(
                "PIPELINE-WINDOW-017",
                format!(
                    "{operation} ordering must include identity field `{}` to prove a total order",
                    key.as_str()
                ),
            ));
        }
    }
    Ok(())
}
