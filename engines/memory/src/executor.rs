mod aggregate;
mod ops;
mod row;
mod window;

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::time::Instant;

use dol_core::diagnostic::{Diagnostic, Result};
use dol_core::expr::Parameters;
use dol_core::model::{ModelDef, Presence};
use dol_core::plan::{
    LogicalExpr, LogicalNode, LogicalPlan, LogicalProjection, PlanId, ProjectionKind,
};
use dol_core::runtime::DynRow;
use dol_core::semantics::Truth;
use dol_core::value::{Datum, Value, validate_datum};
use dol_engine::{DataStream, ExecutionBatch, ExecutionLimits, ExecutionOptions, ExecutionRow};

use crate::store::Tables;
use aggregate::execute_aggregate;
use ops::{JoinRequest, execute_distinct, execute_join, execute_set, execute_sort};
use row::{ScopeRecord, WorkingRow, combined_scopes, evaluation_context};
use window::execute_window;

const MAX_EXISTENTIAL_DEPTH: usize = 256;

pub(crate) fn execute(
    tables: &Tables,
    plan: &LogicalPlan,
    parameters: &Parameters,
    options: &ExecutionOptions,
) -> Result<MemoryStream> {
    options.limits.validate_plan(plan)?;
    if options.batch_rows == 0 {
        return Err(Diagnostic::error(
            "MEMORY-EXEC-001",
            "memory execution batch size must be greater than zero",
        ));
    }

    let mut session = Session::new(tables, plan, parameters, options)?;
    let root = session.execute_subplan(plan.root(), &[])?;
    let rows = root
        .iter()
        .map(|row| row.output.clone())
        .collect::<Vec<_>>();
    enforce_byte_limit(&options.limits, &rows)?;
    session.check_timeout()?;
    Ok(MemoryStream::new(
        plan.output().clone(),
        rows,
        options.batch_rows,
    ))
}

pub(in crate::executor) struct Session<'a> {
    tables: &'a Tables,
    plan: &'a LogicalPlan,
    parameters: &'a Parameters,
    options: &'a ExecutionOptions,
    started: Instant,
    scope_shapes: Vec<Vec<Arc<ModelDef>>>,
    existential_depth: usize,
}

impl<'a> Session<'a> {
    fn new(
        tables: &'a Tables,
        plan: &'a LogicalPlan,
        parameters: &'a Parameters,
        options: &'a ExecutionOptions,
    ) -> Result<Self> {
        Ok(Self {
            tables,
            plan,
            parameters,
            options,
            started: Instant::now(),
            scope_shapes: scope_shapes(plan)?,
            existential_depth: 0,
        })
    }

    fn execute_subplan(
        &mut self,
        root: PlanId,
        outer: &[ScopeRecord],
    ) -> Result<Arc<[WorkingRow]>> {
        self.check_timeout()?;
        let needed = structural_closure(self.plan, root)?;
        let mut results: Vec<Option<Arc<[WorkingRow]>>> = vec![None; self.plan.nodes().len()];

        for index in 0..=root.index() {
            if !needed.get(index).copied().unwrap_or(false) {
                continue;
            }
            self.check_timeout()?;
            let node = self.plan.nodes().get(index).cloned().ok_or_else(|| {
                Diagnostic::error("MEMORY-EXEC-004", "logical node is outside the plan")
            })?;
            let rows = self.execute_stage(&node, &results, outer)?;
            enforce_row_limit(&self.options.limits, rows.len())?;
            enforce_working_byte_limit(&self.options.limits, &rows)?;
            let slot = results.get_mut(index).ok_or_else(|| {
                Diagnostic::error(
                    "MEMORY-EXEC-004",
                    "logical execution result slot is outside the plan",
                )
            })?;
            *slot = Some(rows.into());
        }

        results
            .get(root.index())
            .and_then(Option::as_ref)
            .cloned()
            .ok_or_else(|| {
                Diagnostic::error("MEMORY-EXEC-002", "logical plan root was not executed")
            })
    }

    fn execute_stage(
        &mut self,
        node: &LogicalNode,
        results: &[Option<Arc<[WorkingRow]>>],
        outer: &[ScopeRecord],
    ) -> Result<Vec<WorkingRow>> {
        match node {
            LogicalNode::Source { model, .. } => self.execute_source(model),
            LogicalNode::Filter { input, condition } => {
                let input = input_rows(results, *input)?;
                let mut retained = Vec::with_capacity(input.len());
                let mut retained_bytes = 0_u64;
                for row in input.iter() {
                    self.check_timeout()?;
                    match self.evaluate_expr(condition, row, outer)? {
                        Datum::Value(Value::Truth(Truth::True)) => {
                            self.push_stage_row(&mut retained, &mut retained_bytes, row.clone())?
                        }
                        Datum::Value(Value::Truth(Truth::False | Truth::Unknown)) => {}
                        _ => {
                            return Err(Diagnostic::error(
                                "MEMORY-FILTER-001",
                                "logical filter did not evaluate to DOL truth",
                            ));
                        }
                    }
                }
                Ok(retained)
            }
            LogicalNode::Project { input, projection } => {
                let input = input_rows(results, *input)?;
                let mut projected = Vec::with_capacity(input.len());
                let mut projected_bytes = 0_u64;
                for row in input.iter() {
                    self.check_timeout()?;
                    let datum = self.evaluate_projection(projection, row, outer)?;
                    self.push_stage_row(
                        &mut projected,
                        &mut projected_bytes,
                        WorkingRow {
                            output: ExecutionRow::Value(datum),
                            scopes: Vec::new(),
                        },
                    )?;
                }
                Ok(projected)
            }
            LogicalNode::Aggregate {
                input,
                groups,
                aggregates,
            } => {
                let input = input_rows(results, *input)?;
                execute_aggregate(self, &input, outer, groups.as_deref(), aggregates)
            }
            LogicalNode::Unnest { input, element } => {
                let input = input_rows(results, *input)?;
                self.execute_unnest(&input, element)
            }
            LogicalNode::Window { input, window } => {
                let input = input_rows(results, *input)?;
                execute_window(self, &input, outer, window)
            }
            LogicalNode::Sort { input, keys } => {
                let input = input_rows(results, *input)?;
                execute_sort(self, &input, outer, keys)
            }
            LogicalNode::Distinct { input } => {
                let input = input_rows(results, *input)?;
                execute_distinct(self, &input)
            }
            LogicalNode::Slice {
                input,
                offset,
                limit,
            } => {
                let input = input_rows(results, *input)?;
                let offset = usize::try_from(*offset).unwrap_or(usize::MAX);
                let take = limit
                    .map(|value| usize::try_from(value).unwrap_or(usize::MAX))
                    .unwrap_or(usize::MAX);
                Ok(input.iter().skip(offset).take(take).cloned().collect())
            }
            LogicalNode::Join {
                left,
                right,
                kind,
                condition,
            } => {
                let left_rows = input_rows(results, *left)?;
                let right_rows = input_rows(results, *right)?;
                let left_shape = self.scope_shape(*left)?.to_vec();
                let right_shape = self.scope_shape(*right)?.to_vec();
                execute_join(
                    self,
                    JoinRequest {
                        left: &left_rows,
                        right: &right_rows,
                        left_shape: &left_shape,
                        right_shape: &right_shape,
                        outer,
                        kind: *kind,
                        condition: condition.as_deref(),
                    },
                )
            }
            LogicalNode::Set {
                left,
                right,
                operator,
            } => {
                let left = input_rows(results, *left)?;
                let right = input_rows(results, *right)?;
                execute_set(self, &left, &right, *operator)
            }
            _ => Err(Diagnostic::error(
                "MEMORY-UNSUPPORTED-001",
                "memory engine encountered an unrecognized logical operation",
            )),
        }
    }

    fn execute_source(&self, model: &ModelDef) -> Result<Vec<WorkingRow>> {
        let table = self.tables.get(model.key()).ok_or_else(|| {
            Diagnostic::error(
                "MEMORY-SOURCE-001",
                format!(
                    "model `{}` has no registered memory data",
                    model.key().as_str()
                ),
            )
        })?;
        if table.model.fingerprint() != model.fingerprint() {
            return Err(Diagnostic::error(
                "MEMORY-SOURCE-002",
                "registered memory model does not match logical source semantics",
            ));
        }
        enforce_row_limit(&self.options.limits, table.rows.len())?;
        enforce_dyn_row_byte_limit(&self.options.limits, &table.rows)?;
        Ok(table
            .rows
            .iter()
            .map(|row| WorkingRow {
                output: ExecutionRow::Model(row.clone()),
                scopes: vec![ScopeRecord::Present(row.clone())],
            })
            .collect())
    }

    fn execute_unnest(
        &mut self,
        input: &[WorkingRow],
        element: &dol_core::types::TypeDef,
    ) -> Result<Vec<WorkingRow>> {
        let mut expanded = Vec::new();
        let mut expanded_bytes = 0_u64;
        for row in input {
            self.check_timeout()?;
            let ExecutionRow::Value(datum) = &row.output else {
                return Err(Diagnostic::error(
                    "MEMORY-UNNEST-001",
                    "unnest input is not a logical value row",
                ));
            };
            match datum {
                Datum::Missing | Datum::Null => {}
                Datum::Value(Value::List(values)) => {
                    for value in values {
                        validate_datum(element, Presence::Required, value)?;
                        self.push_stage_row(
                            &mut expanded,
                            &mut expanded_bytes,
                            WorkingRow {
                                output: ExecutionRow::Value(value.clone()),
                                scopes: Vec::new(),
                            },
                        )?;
                    }
                }
                Datum::Value(_) => {
                    return Err(Diagnostic::error(
                        "MEMORY-UNNEST-002",
                        "unnest input did not contain a list value",
                    ));
                }
            }
        }
        Ok(expanded)
    }

    fn evaluate_expr(
        &mut self,
        expression: &LogicalExpr,
        row: &WorkingRow,
        outer: &[ScopeRecord],
    ) -> Result<Datum> {
        let parameters = self.parameters;
        let context = evaluation_context(outer, row, expression.outer_scope_count(), parameters);
        let existential_outer = combined_scopes(
            &outer[..outer.len().min(expression.outer_scope_count())],
            row,
        );
        expression.evaluate_datum_with_exists(&context, &mut |subquery| {
            self.evaluate_exists(subquery, &existential_outer)
        })
    }

    fn evaluate_projection(
        &mut self,
        projection: &LogicalProjection,
        row: &WorkingRow,
        outer: &[ScopeRecord],
    ) -> Result<Datum> {
        let values = projection
            .expressions()
            .iter()
            .map(|expression| self.evaluate_expr(expression, row, outer))
            .collect::<Result<Vec<_>>>()?;
        let datum = match projection.kind() {
            ProjectionKind::Value => values.into_iter().next().ok_or_else(|| {
                Diagnostic::error("MEMORY-PROJECT-001", "scalar projection has no expression")
            })?,
            ProjectionKind::Tuple => Datum::Value(Value::Tuple(values)),
            ProjectionKind::Record => {
                if projection.names().len() != values.len() {
                    return Err(Diagnostic::error(
                        "MEMORY-PROJECT-002",
                        "record projection names do not match expression width",
                    ));
                }
                let fields = projection
                    .names()
                    .iter()
                    .zip(values)
                    .map(|(name, value)| (name.to_string(), value))
                    .collect::<BTreeMap<_, _>>();
                Datum::Value(Value::Record(fields))
            }
        };
        validate_datum(projection.type_def(), Presence::Required, &datum)?;
        Ok(datum)
    }

    fn evaluate_exists(&mut self, subquery: PlanId, outer: &[ScopeRecord]) -> Result<Truth> {
        if self.existential_depth >= MAX_EXISTENTIAL_DEPTH {
            return Err(Diagnostic::error(
                "MEMORY-EXISTS-001",
                "correlated existential nesting exceeds the memory execution safety limit",
            ));
        }
        self.existential_depth = self.existential_depth.saturating_add(1);
        let result = self.execute_subplan(subquery, outer);
        self.existential_depth = self.existential_depth.saturating_sub(1);
        result.map(|rows| {
            if rows.is_empty() {
                Truth::False
            } else {
                Truth::True
            }
        })
    }

    fn scope_shape(&self, id: PlanId) -> Result<&[Arc<ModelDef>]> {
        self.scope_shapes
            .get(id.index())
            .map(Vec::as_slice)
            .ok_or_else(|| {
                Diagnostic::error(
                    "MEMORY-EXEC-007",
                    "logical node has no computed source-scope shape",
                )
            })
    }

    pub(in crate::executor) fn check_timeout(&self) -> Result<()> {
        check_timeout(&self.options.limits, self.started)
    }

    pub(in crate::executor) fn push_stage_row(
        &self,
        rows: &mut Vec<WorkingRow>,
        bytes: &mut u64,
        row: WorkingRow,
    ) -> Result<()> {
        self.check_timeout()?;
        let next_len = rows.len().checked_add(1).ok_or_else(|| {
            Diagnostic::error("ENGINE-LIMIT-004", "memory stage row count overflowed")
        })?;
        enforce_row_limit(&self.options.limits, next_len)?;
        let next_bytes = bytes.saturating_add(row.output.logical_bytes());
        if self
            .options
            .limits
            .max_materialized_bytes
            .is_some_and(|limit| next_bytes > limit)
        {
            return Err(Diagnostic::error(
                "ENGINE-LIMIT-005",
                "memory execution exceeded the configured materialized-byte limit",
            ));
        }
        *bytes = next_bytes;
        rows.push(row);
        Ok(())
    }

    pub(in crate::executor) fn preflight_product(&self, left: usize, right: usize) -> Result<()> {
        self.check_timeout()?;
        let rows = left.checked_mul(right).ok_or_else(|| {
            Diagnostic::error(
                "ENGINE-LIMIT-004",
                "memory join cardinality overflowed before materialization",
            )
        })?;
        enforce_row_limit(&self.options.limits, rows)
    }
}

fn input_rows(results: &[Option<Arc<[WorkingRow]>>], id: PlanId) -> Result<Arc<[WorkingRow]>> {
    results
        .get(id.index())
        .and_then(Option::as_ref)
        .cloned()
        .ok_or_else(|| {
            Diagnostic::error(
                "MEMORY-EXEC-003",
                "logical node references an input that has not been executed",
            )
        })
}

fn structural_closure(plan: &LogicalPlan, root: PlanId) -> Result<Vec<bool>> {
    let mut needed = vec![false; plan.nodes().len()];
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        let slot = needed.get_mut(id.index()).ok_or_else(|| {
            Diagnostic::error(
                "MEMORY-EXEC-005",
                "logical plan contains an invalid structural dependency",
            )
        })?;
        if *slot {
            continue;
        }
        *slot = true;
        let node = plan.nodes().get(id.index()).ok_or_else(|| {
            Diagnostic::error("MEMORY-EXEC-004", "logical node is outside the plan")
        })?;
        push_structural_dependencies(node, &mut stack);
    }
    Ok(needed)
}

fn push_structural_dependencies(node: &LogicalNode, output: &mut Vec<PlanId>) {
    match node {
        LogicalNode::Source { .. } => {}
        LogicalNode::Filter { input, .. }
        | LogicalNode::Project { input, .. }
        | LogicalNode::Aggregate { input, .. }
        | LogicalNode::Unnest { input, .. }
        | LogicalNode::Window { input, .. }
        | LogicalNode::Sort { input, .. }
        | LogicalNode::Distinct { input }
        | LogicalNode::Slice { input, .. } => output.push(*input),
        LogicalNode::Join { left, right, .. } | LogicalNode::Set { left, right, .. } => {
            output.push(*left);
            output.push(*right);
        }
        _ => {}
    }
}

fn scope_shapes(plan: &LogicalPlan) -> Result<Vec<Vec<Arc<ModelDef>>>> {
    let mut shapes: Vec<Vec<Arc<ModelDef>>> = Vec::with_capacity(plan.nodes().len());
    for node in plan.nodes() {
        let shape = match node {
            LogicalNode::Source { model, .. } => vec![Arc::new((**model).clone())],
            LogicalNode::Filter { input, .. }
            | LogicalNode::Window { input, .. }
            | LogicalNode::Sort { input, .. }
            | LogicalNode::Distinct { input }
            | LogicalNode::Slice { input, .. } => shape_for(&shapes, *input)?.to_vec(),
            LogicalNode::Project { .. }
            | LogicalNode::Aggregate { .. }
            | LogicalNode::Unnest { .. } => Vec::new(),
            LogicalNode::Join { left, right, .. } => {
                let left = shape_for(&shapes, *left)?;
                let right = shape_for(&shapes, *right)?;
                let mut shape = Vec::with_capacity(left.len() + right.len());
                shape.extend_from_slice(left);
                shape.extend_from_slice(right);
                shape
            }
            LogicalNode::Set { left, .. } => shape_for(&shapes, *left)?.to_vec(),
            _ => {
                return Err(Diagnostic::error(
                    "MEMORY-EXEC-006",
                    "memory engine cannot derive scopes for an unrecognized logical operation",
                ));
            }
        };
        shapes.push(shape);
    }
    Ok(shapes)
}

fn shape_for(shapes: &[Vec<Arc<ModelDef>>], id: PlanId) -> Result<&[Arc<ModelDef>]> {
    shapes.get(id.index()).map(Vec::as_slice).ok_or_else(|| {
        Diagnostic::error(
            "MEMORY-EXEC-005",
            "logical plan scope shape references a non-topological input",
        )
    })
}

fn enforce_row_limit(limits: &ExecutionLimits, rows: usize) -> Result<()> {
    if let Some(max_rows) = limits.max_materialized_rows
        && u64::try_from(rows).unwrap_or(u64::MAX) > max_rows
    {
        return Err(Diagnostic::error(
            "ENGINE-LIMIT-004",
            "memory execution exceeded the configured row limit",
        ));
    }
    Ok(())
}

fn enforce_dyn_row_byte_limit(limits: &ExecutionLimits, rows: &[DynRow]) -> Result<()> {
    let Some(max_bytes) = limits.max_materialized_bytes else {
        return Ok(());
    };
    let bytes = rows.iter().fold(0_u64, |total, row| {
        total.saturating_add(row.logical_bytes())
    });
    if bytes > max_bytes {
        return Err(Diagnostic::error(
            "ENGINE-LIMIT-005",
            "memory execution exceeded the configured materialized-byte limit",
        ));
    }
    Ok(())
}

fn enforce_working_byte_limit(limits: &ExecutionLimits, rows: &[WorkingRow]) -> Result<()> {
    let Some(max_bytes) = limits.max_materialized_bytes else {
        return Ok(());
    };
    let bytes = rows.iter().fold(0_u64, |total, row| {
        total.saturating_add(row.output.logical_bytes())
    });
    if bytes > max_bytes {
        return Err(Diagnostic::error(
            "ENGINE-LIMIT-005",
            "memory execution exceeded the configured materialized-byte limit",
        ));
    }
    Ok(())
}

fn enforce_byte_limit(limits: &ExecutionLimits, rows: &[ExecutionRow]) -> Result<()> {
    let Some(max_bytes) = limits.max_materialized_bytes else {
        return Ok(());
    };
    let bytes = rows.iter().fold(0_u64, |total, row| {
        total.saturating_add(row.logical_bytes())
    });
    if bytes > max_bytes {
        return Err(Diagnostic::error(
            "ENGINE-LIMIT-005",
            "memory execution exceeded the configured result-byte limit",
        ));
    }
    Ok(())
}

fn check_timeout(limits: &ExecutionLimits, started: Instant) -> Result<()> {
    if let Some(timeout) = limits.timeout
        && started.elapsed() > timeout
    {
        return Err(Diagnostic::error(
            "ENGINE-LIMIT-006",
            "memory execution exceeded the configured timeout",
        ));
    }
    Ok(())
}

/// Eagerly-produced rows exposed through the engine pull-stream contract.
#[derive(Debug)]
pub struct MemoryStream {
    output: dol_core::plan::PlanOutput,
    rows: VecDeque<ExecutionRow>,
    batch_rows: usize,
    cancelled: bool,
}

impl MemoryStream {
    fn new(output: dol_core::plan::PlanOutput, rows: Vec<ExecutionRow>, batch_rows: usize) -> Self {
        Self {
            output,
            rows: rows.into(),
            batch_rows,
            cancelled: false,
        }
    }
}

impl DataStream for MemoryStream {
    fn next_batch(&mut self) -> Result<Option<ExecutionBatch>> {
        if self.cancelled || self.rows.is_empty() {
            return Ok(None);
        }
        let count = self.batch_rows.min(self.rows.len());
        let rows = self
            .rows
            .drain(..count)
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Ok(Some(ExecutionBatch::new(self.output.clone(), rows)))
    }

    fn cancel(&mut self) {
        self.cancelled = true;
        self.rows.clear();
    }
}
