use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Take};
use std::sync::Arc;
use std::time::Instant;

use dol_core::diagnostic::{Diagnostic, Result};
use dol_core::expr::{EvalContext, Parameters};
use dol_core::model::{ModelDef, Presence};
use dol_core::plan::{
    LogicalExpr, LogicalNode, LogicalPlan, LogicalProjection, PlanOutput, ProjectionKind,
};
use dol_core::runtime::DynRow;
use dol_core::semantics::Truth;
use dol_core::types::TypeDef;
use dol_core::value::{Datum, Value, validate_datum};
use dol_engine::{DataStream, ExecutionBatch, ExecutionLimits, ExecutionOptions, ExecutionRow};

use crate::JsonlLimits;
use crate::decode::decode_model_line;
use crate::source::{JsonlSource, Sources};

#[derive(Debug, Clone)]
enum StreamOp {
    Filter(LogicalExpr),
    Project(LogicalProjection),
    Unnest(TypeDef),
    Slice {
        offset_remaining: u64,
        limit_remaining: Option<u64>,
    },
}

impl StreamOp {
    fn exhausted(&self) -> bool {
        matches!(
            self,
            Self::Slice {
                limit_remaining: Some(0),
                ..
            }
        )
    }
}

#[derive(Debug)]
struct CompiledPlan {
    source: JsonlSource,
    ops: Vec<StreamOp>,
    output: PlanOutput,
}

#[derive(Debug, Clone)]
struct StreamRow {
    output: ExecutionRow,
}

impl StreamRow {
    fn from_model(row: DynRow) -> Self {
        Self {
            output: ExecutionRow::Model(row),
        }
    }

    fn record(&self) -> Option<&DynRow> {
        match &self.output {
            ExecutionRow::Model(row) => Some(row),
            _ => None,
        }
    }
}

/// Lazy pull stream over one bounded JSONL source.
#[derive(Debug)]
pub struct JsonlStream {
    reader: BufReader<Take<File>>,
    model: Arc<ModelDef>,
    ops: Vec<StreamOp>,
    parameters: Parameters,
    output: PlanOutput,
    execution_limits: ExecutionLimits,
    jsonl_limits: JsonlLimits,
    batch_rows: usize,
    pending: VecDeque<StreamRow>,
    line_buffer: Vec<u8>,
    line_number: u64,
    scanned_rows: u64,
    started: Instant,
    done: bool,
    cancelled: bool,
}

impl JsonlStream {
    pub(crate) fn open(
        sources: &Sources,
        plan: &LogicalPlan,
        parameters: &Parameters,
        options: &ExecutionOptions,
        jsonl_limits: &JsonlLimits,
    ) -> Result<Self> {
        options.limits.validate_plan(plan)?;
        jsonl_limits.validate()?;
        if options.batch_rows == 0 {
            return Err(Diagnostic::error(
                "JSONL-EXEC-001",
                "JSONL execution batch size must be greater than zero",
            ));
        }

        let compiled = compile_plan(sources, plan)?;
        let file = compiled.source.open_file()?;
        let source_bytes = file
            .metadata()
            .map_err(|error| {
                Diagnostic::error(
                    "JSONL-SOURCE-005",
                    format!("cannot inspect open JSONL source: {error}"),
                )
            })?
            .len();
        if source_bytes > jsonl_limits.max_source_bytes {
            return Err(Diagnostic::error(
                "JSONL-LIMIT-001",
                "JSONL source exceeds the configured source-byte limit",
            ));
        }

        let capacity = jsonl_limits.max_line_bytes.clamp(1, 64 * 1024);
        Ok(Self {
            reader: BufReader::with_capacity(capacity, file.take(source_bytes)),
            model: compiled.source.model,
            ops: compiled.ops,
            parameters: parameters.clone(),
            output: compiled.output,
            execution_limits: options.limits.clone(),
            jsonl_limits: jsonl_limits.clone(),
            batch_rows: options.batch_rows,
            pending: VecDeque::new(),
            line_buffer: Vec::new(),
            line_number: 0,
            scanned_rows: 0,
            started: Instant::now(),
            done: false,
            cancelled: false,
        })
    }

    fn fill_pending(&mut self) -> Result<()> {
        while self.pending.is_empty() && !self.done {
            self.check_timeout()?;
            if self.ops.iter().any(StreamOp::exhausted) {
                self.done = true;
                break;
            }
            if !read_bounded_line(
                &mut self.reader,
                &mut self.line_buffer,
                self.jsonl_limits.max_line_bytes,
            )? {
                self.done = true;
                break;
            }
            self.line_number = self.line_number.saturating_add(1);
            self.scanned_rows = self.scanned_rows.saturating_add(1);
            if self.scanned_rows > self.jsonl_limits.max_scanned_rows {
                return Err(Diagnostic::error(
                    "JSONL-LIMIT-003",
                    "JSONL execution exceeds the configured scanned-row limit",
                ));
            }
            let row = {
                let line = std::str::from_utf8(&self.line_buffer).map_err(|error| {
                    Diagnostic::error(
                        "JSONL-DECODE-007",
                        format!(
                            "JSONL line {} is not valid UTF-8: {error}",
                            self.line_number
                        ),
                    )
                })?;
                let line = line.strip_suffix('\r').unwrap_or(line);
                if line.is_empty() {
                    return Err(Diagnostic::error(
                        "JSONL-DECODE-002",
                        format!("JSONL line {} is empty", self.line_number),
                    ));
                }
                decode_model_line(
                    self.model.clone(),
                    line,
                    &self.jsonl_limits,
                    self.line_number,
                )?
            };
            let source_row = StreamRow::from_model(row);
            enforce_rows(&self.execution_limits, core::slice::from_ref(&source_row))?;
            let produced = apply_ops(
                &mut self.ops,
                source_row,
                &self.parameters,
                &self.execution_limits,
            )?;
            self.check_timeout()?;
            self.pending.extend(produced);
        }
        Ok(())
    }

    fn check_timeout(&self) -> Result<()> {
        if let Some(timeout) = self.execution_limits.timeout
            && self.started.elapsed() > timeout
        {
            return Err(Diagnostic::error(
                "ENGINE-LIMIT-006",
                "JSONL execution exceeded the configured timeout",
            ));
        }
        Ok(())
    }
}

impl DataStream for JsonlStream {
    fn next_batch(&mut self) -> Result<Option<ExecutionBatch>> {
        if self.cancelled || (self.done && self.pending.is_empty()) {
            return Ok(None);
        }
        self.check_timeout()?;

        let materialized_row_limit = self
            .execution_limits
            .max_materialized_rows
            .and_then(|limit| usize::try_from(limit).ok())
            .unwrap_or(usize::MAX);
        let target_rows = self.batch_rows.min(materialized_row_limit.max(1));
        let mut rows = Vec::with_capacity(target_rows.min(1024));
        let mut bytes = 0_u64;
        while rows.len() < target_rows {
            self.fill_pending()?;
            let Some(next) = self.pending.front() else {
                break;
            };
            let row_bytes = next.output.logical_bytes();
            if let Some(max_bytes) = self.execution_limits.max_materialized_bytes {
                if row_bytes > max_bytes {
                    return Err(Diagnostic::error(
                        "ENGINE-LIMIT-005",
                        "one JSONL result row exceeds the configured materialized-byte limit",
                    ));
                }
                if !rows.is_empty() && bytes.saturating_add(row_bytes) > max_bytes {
                    break;
                }
            }
            let next = self.pending.pop_front().ok_or_else(|| {
                Diagnostic::error(
                    "JSONL-EXEC-004",
                    "pending JSONL row disappeared during batch assembly",
                )
            })?;
            bytes = bytes.saturating_add(row_bytes);
            rows.push(next.output);
        }

        if rows.is_empty() {
            Ok(None)
        } else {
            Ok(Some(ExecutionBatch::new(
                self.output.clone(),
                rows.into_boxed_slice(),
            )))
        }
    }

    fn cancel(&mut self) {
        self.cancelled = true;
        self.pending.clear();
    }
}

fn compile_plan(sources: &Sources, plan: &LogicalPlan) -> Result<CompiledPlan> {
    let mut current = plan.root();
    let mut reversed = Vec::new();
    let source_model = loop {
        let node = plan.nodes().get(current.index()).ok_or_else(|| {
            Diagnostic::error("JSONL-EXEC-002", "logical plan contains an invalid node id")
        })?;
        match node {
            LogicalNode::Source { model, .. } => break *model,
            LogicalNode::Filter { input, condition } => {
                if !condition.existential_dependencies().is_empty() {
                    return unsupported("existential expressions");
                }
                reversed.push(StreamOp::Filter((**condition).clone()));
                current = *input;
            }
            LogicalNode::Project { input, projection } => {
                if projection
                    .expressions()
                    .iter()
                    .any(|expression| !expression.existential_dependencies().is_empty())
                {
                    return unsupported("existential expressions");
                }
                reversed.push(StreamOp::Project((**projection).clone()));
                current = *input;
            }
            LogicalNode::Unnest { input, element } => {
                reversed.push(StreamOp::Unnest(element.clone()));
                current = *input;
            }
            LogicalNode::Slice {
                input,
                offset,
                limit,
            } => {
                reversed.push(StreamOp::Slice {
                    offset_remaining: *offset,
                    limit_remaining: *limit,
                });
                current = *input;
            }
            LogicalNode::Aggregate { .. }
            | LogicalNode::Window { .. }
            | LogicalNode::Sort { .. }
            | LogicalNode::Distinct { .. }
            | LogicalNode::Join { .. }
            | LogicalNode::Set { .. } => return unsupported("materializing or multi-source node"),
            _ => return unsupported("unrecognized logical node"),
        }
    };

    reversed.reverse();
    let source = sources.get(source_model.key()).ok_or_else(|| {
        Diagnostic::error(
            "JSONL-SOURCE-006",
            format!(
                "model `{}` has no configured JSONL source",
                source_model.key().as_str()
            ),
        )
    })?;
    if source.model.fingerprint() != source_model.fingerprint() {
        return Err(Diagnostic::error(
            "JSONL-SOURCE-007",
            "configured JSONL model semantics differ from the logical source",
        ));
    }

    Ok(CompiledPlan {
        source: source.clone(),
        ops: reversed,
        output: plan.output().clone(),
    })
}

fn apply_ops(
    ops: &mut [StreamOp],
    initial: StreamRow,
    parameters: &Parameters,
    limits: &ExecutionLimits,
) -> Result<Vec<StreamRow>> {
    let mut rows = vec![initial];
    enforce_rows(limits, &rows)?;
    for op in ops {
        let mut next = Vec::new();
        for row in rows {
            apply_op(op, row, parameters, limits, &mut next)?;
        }
        enforce_rows(limits, &next)?;
        rows = next;
        if rows.is_empty() {
            break;
        }
    }
    Ok(rows)
}

fn apply_op(
    op: &mut StreamOp,
    row: StreamRow,
    parameters: &Parameters,
    limits: &ExecutionLimits,
    output: &mut Vec<StreamRow>,
) -> Result<()> {
    match op {
        StreamOp::Filter(condition) => match evaluate_expr(condition, &row, parameters)? {
            Datum::Value(Value::Truth(Truth::True)) => output.push(row),
            Datum::Value(Value::Truth(Truth::False | Truth::Unknown)) => {}
            _ => {
                return Err(Diagnostic::error(
                    "JSONL-FILTER-001",
                    "logical filter did not evaluate to DOL truth",
                ));
            }
        },
        StreamOp::Project(projection) => {
            let datum = evaluate_projection(projection, &row, parameters)?;
            output.push(StreamRow {
                output: ExecutionRow::Value(datum),
            });
        }
        StreamOp::Unnest(element) => {
            let ExecutionRow::Value(datum) = &row.output else {
                return Err(Diagnostic::error(
                    "JSONL-UNNEST-001",
                    "unnest input is not a logical value row",
                ));
            };
            match datum {
                Datum::Missing | Datum::Null => {}
                Datum::Value(Value::List(values)) => {
                    for value in values {
                        validate_datum(element, Presence::Required, value)?;
                        output.push(StreamRow {
                            output: ExecutionRow::Value(value.clone()),
                        });
                        enforce_rows(limits, output)?;
                    }
                }
                Datum::Value(_) => {
                    return Err(Diagnostic::error(
                        "JSONL-UNNEST-002",
                        "unnest input did not contain a list value",
                    ));
                }
            }
        }
        StreamOp::Slice {
            offset_remaining,
            limit_remaining,
        } => {
            if *offset_remaining > 0 {
                *offset_remaining = offset_remaining.saturating_sub(1);
            } else {
                match limit_remaining {
                    Some(0) => {}
                    Some(remaining) => {
                        *remaining = remaining.saturating_sub(1);
                        output.push(row);
                    }
                    None => output.push(row),
                }
            }
        }
    }
    Ok(())
}

fn evaluate_expr(
    expression: &LogicalExpr,
    row: &StreamRow,
    parameters: &Parameters,
) -> Result<Datum> {
    let context = eval_context(expression, row, parameters)?;
    expression.evaluate_datum(&context)
}

fn evaluate_projection(
    projection: &LogicalProjection,
    row: &StreamRow,
    parameters: &Parameters,
) -> Result<Datum> {
    let values = projection
        .expressions()
        .iter()
        .map(|expression| evaluate_expr(expression, row, parameters))
        .collect::<Result<Vec<_>>>()?;
    let datum = match projection.kind() {
        ProjectionKind::Value => values.into_iter().next().ok_or_else(|| {
            Diagnostic::error("JSONL-PROJECT-001", "scalar projection has no expression")
        })?,
        ProjectionKind::Tuple => Datum::Value(Value::Tuple(values)),
        ProjectionKind::Record => {
            if projection.names().len() != values.len() {
                return Err(Diagnostic::error(
                    "JSONL-PROJECT-002",
                    "record projection names do not match expression width",
                ));
            }
            let fields = projection
                .names()
                .iter()
                .zip(values)
                .map(|(name, value)| (name.to_string(), value))
                .collect();
            Datum::Value(Value::Record(fields))
        }
    };
    validate_datum(projection.type_def(), Presence::Required, &datum)?;
    Ok(datum)
}

fn eval_context<'a>(
    expression: &LogicalExpr,
    row: &'a StreamRow,
    parameters: &'a Parameters,
) -> Result<EvalContext<'a>> {
    match expression.scope_count() {
        0 => Ok(EvalContext::new().with_parameters(parameters)),
        1 => row
            .record()
            .map(|record| EvalContext::single(record).with_parameters(parameters))
            .ok_or_else(|| {
                Diagnostic::error(
                    "JSONL-EXPR-001",
                    "expression requires a model scope after that scope has been closed",
                )
            }),
        _ => Err(Diagnostic::error(
            "JSONL-EXPR-002",
            "incremental JSONL execution supports at most one local model scope",
        )),
    }
}

fn enforce_rows(limits: &ExecutionLimits, rows: &[StreamRow]) -> Result<()> {
    if let Some(max_rows) = limits.max_materialized_rows
        && u64::try_from(rows.len()).unwrap_or(u64::MAX) > max_rows
    {
        return Err(Diagnostic::error(
            "ENGINE-LIMIT-004",
            "JSONL execution exceeded the configured materialized-row limit",
        ));
    }
    if let Some(max_bytes) = limits.max_materialized_bytes {
        let bytes = rows.iter().fold(0_u64, |total, row| {
            total.saturating_add(row.output.logical_bytes())
        });
        if bytes > max_bytes {
            return Err(Diagnostic::error(
                "ENGINE-LIMIT-005",
                "JSONL execution exceeded the configured materialized-byte limit",
            ));
        }
    }
    Ok(())
}

fn unsupported<T>(operation: &str) -> Result<T> {
    Err(Diagnostic::error(
        "JSONL-UNSUPPORTED-001",
        format!("JSONL incremental executor does not support {operation}"),
    ))
}

fn read_bounded_line<R>(reader: &mut R, buffer: &mut Vec<u8>, max_line_bytes: usize) -> Result<bool>
where
    R: BufRead,
{
    buffer.clear();
    loop {
        let available = reader.fill_buf().map_err(|error| {
            Diagnostic::error(
                "JSONL-SOURCE-005",
                format!("failed while reading JSONL source: {error}"),
            )
        })?;
        if available.is_empty() {
            return Ok(!buffer.is_empty());
        }

        let newline = available.iter().position(|byte| *byte == b'\n');
        let content_len = newline.unwrap_or(available.len());
        if buffer.len().saturating_add(content_len) > max_line_bytes {
            return Err(Diagnostic::error(
                "JSONL-LIMIT-002",
                "JSONL record exceeds the configured line-byte limit",
            ));
        }
        buffer.extend_from_slice(&available[..content_len]);
        let consumed = content_len + usize::from(newline.is_some());
        reader.consume(consumed);
        if newline.is_some() {
            return Ok(true);
        }
    }
}
