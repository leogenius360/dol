//! Aggregate-specific logical compilation and semantic validation.

use crate::analytics::{
    AggregateInputMode, AggregateKind, AggregateSelectionKind, AggregateSelectionSpec,
};
use crate::diagnostic::{Diagnostic, Result};
use crate::expr::{BindContext, prepare_expression};
use crate::limits::PipelineLimits;
use crate::plan::{LogicalAggregate, LogicalAggregateSelection, LogicalExpr, LogicalProjection};
use crate::types::{DataType, TypeDef, TypeShape};

pub(super) fn validate_group_projection(groups: &LogicalProjection) -> Result<()> {
    let properties = groups.type_def().properties();
    if !properties.equality || !properties.keyable {
        return Err(Diagnostic::error(
            "PIPELINE-GROUP-001",
            format!(
                "group key type `{}` requires stable equality and key semantics",
                groups.type_def().key().as_str()
            ),
        ));
    }
    Ok(())
}

pub(super) fn compile_aggregate_selection(
    selection: &AggregateSelectionSpec,
    context: &BindContext<'_>,
    limits: PipelineLimits,
) -> Result<LogicalAggregateSelection> {
    let mut aggregates = Vec::with_capacity(selection.aggregates.len());
    for aggregate in &selection.aggregates {
        let input = aggregate
            .input
            .as_ref()
            .map(|expression| {
                prepare_expression(expression, context, limits.expression)
                    .map(LogicalExpr::from_prepared)
            })
            .transpose()?;
        validate_aggregate(
            aggregate.kind,
            aggregate.input_mode,
            input.as_ref(),
            &aggregate.ty,
        )?;
        aggregates.push(LogicalAggregate::new(
            aggregate.kind,
            input,
            aggregate.ty.clone(),
        ));
    }

    match selection.kind {
        AggregateSelectionKind::Value => {
            if aggregates.len() != 1 || aggregates[0].type_def() != &selection.ty {
                return Err(Diagnostic::error(
                    "PIPELINE-AGG-001",
                    "single aggregate selection output type does not match its aggregate",
                ));
            }
        }
        AggregateSelectionKind::Tuple => match selection.ty.shape() {
            TypeShape::Tuple(elements)
                if elements.len() == aggregates.len()
                    && elements
                        .iter()
                        .zip(&aggregates)
                        .all(|(expected, aggregate)| expected == aggregate.type_def()) => {}
            _ => {
                return Err(Diagnostic::error(
                    "PIPELINE-AGG-002",
                    "aggregate tuple output type does not match its component aggregates",
                ));
            }
        },
    }

    Ok(LogicalAggregateSelection::new(
        selection.kind,
        aggregates.into_boxed_slice(),
        selection.ty.clone(),
    ))
}

fn validate_aggregate(
    kind: AggregateKind,
    input_mode: AggregateInputMode,
    input: Option<&LogicalExpr>,
    output: &TypeDef,
) -> Result<()> {
    match kind {
        AggregateKind::CountRows => {
            if input.is_some() || output != &u64::type_def() {
                return Err(Diagnostic::error(
                    "PIPELINE-AGG-003",
                    "row count must have no input expression and must produce dol/u64",
                ));
            }
        }
        AggregateKind::CountPresent => {
            if input.is_none() || output != &u64::type_def() {
                return Err(Diagnostic::error(
                    "PIPELINE-AGG-003",
                    "present-value count requires one input and must produce dol/u64",
                ));
            }
        }
        AggregateKind::Sum => {
            let input = require_aggregate_input(input, "sum")?;
            reject_implicit_nullable_aggregate(input_mode, input, "sum", "sum_present")?;
            if !input.type_def().properties().exact_numeric {
                return Err(Diagnostic::error(
                    "PIPELINE-AGG-004",
                    format!(
                        "sum requires exact numeric semantics; type `{}` is not an exact numeric type",
                        input.type_def().key().as_str()
                    ),
                ));
            }
            validate_nullable_aggregate_output(input, output, "sum")?;
        }
        AggregateKind::Min | AggregateKind::Max => {
            let operation = if kind == AggregateKind::Min {
                "min"
            } else {
                "max"
            };
            let present = if kind == AggregateKind::Min {
                "min_present"
            } else {
                "max_present"
            };
            let input = require_aggregate_input(input, operation)?;
            reject_implicit_nullable_aggregate(input_mode, input, operation, present)?;
            let properties = input.type_def().properties();
            if !properties.ordering || !properties.keyable {
                return Err(Diagnostic::error(
                    "PIPELINE-AGG-005",
                    format!(
                        "{operation} requires stable ordering and key semantics; type `{}` does not provide both",
                        input.type_def().key().as_str()
                    ),
                ));
            }
            validate_nullable_aggregate_output(input, output, operation)?;
        }
    }
    Ok(())
}

fn require_aggregate_input<'a>(
    input: Option<&'a LogicalExpr>,
    operation: &str,
) -> Result<&'a LogicalExpr> {
    input.ok_or_else(|| {
        Diagnostic::error(
            "PIPELINE-AGG-003",
            format!("{operation} requires one input expression"),
        )
    })
}

fn reject_implicit_nullable_aggregate(
    input_mode: AggregateInputMode,
    input: &LogicalExpr,
    operation: &str,
    present_operation: &str,
) -> Result<()> {
    if input_mode == AggregateInputMode::NonNull && input.type_def().is_nullable() {
        return Err(Diagnostic::error(
            "PIPELINE-AGG-006",
            format!(
                "{operation} received a nullable source; use `{present_operation}` to state null/missing handling explicitly"
            ),
        ));
    }
    Ok(())
}

fn validate_nullable_aggregate_output(
    input: &LogicalExpr,
    output: &TypeDef,
    operation: &str,
) -> Result<()> {
    let expected = input.type_def().clone().nullable();
    if output != &expected {
        return Err(Diagnostic::error(
            "PIPELINE-AGG-007",
            format!("{operation} output type does not match the nullable input semantic type"),
        ));
    }
    Ok(())
}
