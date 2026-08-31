use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::diagnostic::{Diagnostic, Result};
use crate::expr::{
    BindContext, ExistentialRef, PreparedExistential, prepare_expression,
    prepare_expression_with_existentials,
};
use crate::limits::{DefinitionLimits, PipelineLimits};
use crate::model::ModelDef;
use crate::plan::{
    JoinKind, LogicalExpr, LogicalNode, LogicalPlan, LogicalProjection, PlanBuilder, PlanId,
    PlanOutput, ProjectionKind, SetOperator, SortExpr, fingerprint_aggregate_stage,
    fingerprint_expression_stage, fingerprint_join_stage, fingerprint_projection_stage,
    fingerprint_set_stage, fingerprint_slice_stage, fingerprint_sort_stage, fingerprint_source,
    fingerprint_unary_stage,
};
use crate::types::{TypeDef, TypeShape, validate_type, validate_type_universe};

use super::{PipelineNode, projection::ProjectionSpec};

mod advanced;
mod aggregate;
use advanced::{compile_unnest, compile_window};
use aggregate::{compile_aggregate_selection, validate_group_projection};

pub(crate) fn compile(node: &Arc<PipelineNode>, limits: PipelineLimits) -> Result<LogicalPlan> {
    let stage_count = PipelineNode::stage_count(node);
    if stage_count > limits.max_nodes {
        return Err(Diagnostic::error(
            "PIPELINE-LIMIT-002",
            format!("pipeline has more than {} logical nodes", limits.max_nodes),
        ));
    }

    let mut builder = PlanBuilder::new();
    let result = compile_pipeline(node, &mut builder, limits, &[])?;
    builder.finish(result.id, result.output.plan_output())
}

fn compile_pipeline(
    node: &Arc<PipelineNode>,
    builder: &mut PlanBuilder,
    limits: PipelineLimits,
    outer_scopes: &[Scope],
) -> Result<Compiled> {
    let nodes = collect_nodes(node, limits.max_nodes)?;
    let mut compiled = HashMap::with_capacity(nodes.len());

    for node in nodes {
        let result = compile_node(&node, &compiled, builder, limits, outer_scopes)?;
        compiled.insert(Arc::as_ptr(&node), result);
    }

    compiled.get(&Arc::as_ptr(node)).cloned().ok_or_else(|| {
        Diagnostic::error("PIPELINE-PLAN-002", "pipeline contains no semantic source")
    })
}

#[derive(Clone)]
struct Compiled {
    id: PlanId,
    scopes: Vec<Scope>,
    output: Output,
}

#[derive(Clone)]
struct Scope {
    model: &'static ModelDef,
    alias: Option<Arc<str>>,
    nullable: bool,
    identity_unique: bool,
}

#[derive(Clone)]
enum Output {
    Model(&'static ModelDef),
    Value(TypeDef),
    Product(Box<[Output]>),
    Nullable(Box<Output>),
}

impl Output {
    fn plan_output(&self) -> PlanOutput {
        match self {
            Self::Model(model) => PlanOutput::Model(Box::new((*model).clone())),
            Self::Value(ty) => PlanOutput::Value(ty.clone()),
            Self::Product(outputs) => PlanOutput::Product(
                outputs
                    .iter()
                    .map(Self::plan_output)
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
            ),
            Self::Nullable(output) => PlanOutput::Nullable(Box::new(output.plan_output())),
        }
    }

    fn validate_equality(&self, operation: &str) -> Result<()> {
        match self {
            Self::Model(model) => {
                if let Some(field) = model
                    .fields()
                    .iter()
                    .find(|field| !field.ty().properties().equality)
                {
                    return Err(Diagnostic::error(
                        "PIPELINE-TYPE-002",
                        format!(
                            "model `{}` cannot use {operation} because field `{}` has no equality semantics",
                            model.key().as_str(),
                            field.key().as_str()
                        ),
                    ));
                }
                Ok(())
            }
            Self::Value(ty) if ty.properties().equality => Ok(()),
            Self::Value(ty) => Err(Diagnostic::error(
                "PIPELINE-TYPE-002",
                format!(
                    "pipeline output type `{}` has no equality semantics required by {operation}",
                    ty.key().as_str()
                ),
            )),
            Self::Product(outputs) => {
                for output in outputs {
                    output.validate_equality(operation)?;
                }
                Ok(())
            }
            Self::Nullable(output) => output.validate_equality(operation),
        }
    }

    fn semantically_matches(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Model(left), Self::Model(right)) => left.fingerprint() == right.fingerprint(),
            (Self::Value(left), Self::Value(right)) => left == right,
            (Self::Product(left), Self::Product(right)) => {
                left.len() == right.len()
                    && left
                        .iter()
                        .zip(right.iter())
                        .all(|(left, right)| left.semantically_matches(right))
            }
            (Self::Nullable(left), Self::Nullable(right)) => left.semantically_matches(right),
            _ => false,
        }
    }

    fn nullable(self) -> Self {
        match self {
            output @ Self::Nullable(_) => output,
            output => Self::Nullable(Box::new(output)),
        }
    }
}

fn collect_nodes(root: &Arc<PipelineNode>, max_nodes: usize) -> Result<Vec<Arc<PipelineNode>>> {
    let mut order = Vec::new();
    let mut seen = HashSet::new();
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

fn compile_node(
    node: &Arc<PipelineNode>,
    compiled: &HashMap<*const PipelineNode, Compiled>,
    builder: &mut PlanBuilder,
    limits: PipelineLimits,
    outer_scopes: &[Scope],
) -> Result<Compiled> {
    match node.as_ref() {
        PipelineNode::Source(source) => compile_source(source, builder),
        PipelineNode::Filter { input, condition } => {
            let input = require_compiled(compiled, input)?;
            let binding_scopes = combine_scopes(outer_scopes, &input.scopes)?;
            let context = bind_context(&binding_scopes);
            let mut resolve_existential = |exists: &ExistentialRef| {
                compile_existential(builder, limits, &binding_scopes, exists)
            };
            let condition = LogicalExpr::from_prepared(prepare_expression_with_existentials(
                condition,
                &context,
                limits.expression,
                outer_scopes.len(),
                &mut resolve_existential,
            )?);
            let fingerprint = fingerprint_expression_stage(
                b"pipeline/filter/v2",
                builder.fingerprint(input.id)?,
                &condition,
            );
            let id = builder.push(
                LogicalNode::Filter {
                    input: input.id,
                    condition: Box::new(condition),
                },
                fingerprint,
            )?;
            Ok(Compiled { id, ..input })
        }
        PipelineNode::Project { input, projection } => {
            let input = require_compiled(compiled, input)?;
            let context = bind_context(&input.scopes);
            let projection = compile_projection(projection, &context, limits)?;
            let output = Output::Value(projection.type_def().clone());
            let fingerprint =
                fingerprint_projection_stage(builder.fingerprint(input.id)?, &projection);
            let id = builder.push(
                LogicalNode::Project {
                    input: input.id,
                    projection: Box::new(projection),
                },
                fingerprint,
            )?;
            Ok(Compiled {
                id,
                scopes: Vec::new(),
                output,
            })
        }
        PipelineNode::Aggregate {
            input,
            groups,
            aggregates,
        } => {
            let input = require_compiled(compiled, input)?;
            let context = bind_context(&input.scopes);
            let groups = groups
                .as_ref()
                .map(|groups| compile_projection(groups, &context, limits))
                .transpose()?;
            if let Some(groups) = &groups {
                validate_group_projection(groups)?;
            }
            let aggregates = compile_aggregate_selection(aggregates, &context, limits)?;
            let output_ty = groups.as_ref().map_or_else(
                || aggregates.type_def().clone(),
                |groups| TypeDef::tuple([groups.type_def().clone(), aggregates.type_def().clone()]),
            );
            let fingerprint = fingerprint_aggregate_stage(
                builder.fingerprint(input.id)?,
                groups.as_ref(),
                &aggregates,
            );
            let id = builder.push(
                LogicalNode::Aggregate {
                    input: input.id,
                    groups: groups.map(Box::new),
                    aggregates: Box::new(aggregates),
                },
                fingerprint,
            )?;
            Ok(Compiled {
                id,
                scopes: Vec::new(),
                output: Output::Value(output_ty),
            })
        }
        PipelineNode::Unnest { input } => compile_unnest(compiled, builder, input),
        PipelineNode::Window { input, window } => {
            compile_window(compiled, builder, limits, input, window)
        }
        PipelineNode::Sort {
            input,
            expression,
            direction,
        } => {
            let input = require_compiled(compiled, input)?;
            let context = bind_context(&input.scopes);
            let expression = LogicalExpr::from_prepared(prepare_expression(
                expression,
                &context,
                limits.expression,
            )?);
            if !expression.type_def().properties().ordering {
                return Err(Diagnostic::error(
                    "PIPELINE-TYPE-001",
                    format!(
                        "type `{}` has no ordering semantics required by order_by",
                        expression.type_def().key().as_str()
                    ),
                ));
            }
            let keys = vec![SortExpr::new(expression, *direction)].into_boxed_slice();
            let fingerprint = fingerprint_sort_stage(builder.fingerprint(input.id)?, &keys);
            let id = builder.push(
                LogicalNode::Sort {
                    input: input.id,
                    keys,
                },
                fingerprint,
            )?;
            Ok(Compiled { id, ..input })
        }
        PipelineNode::Distinct { input } => {
            let input = require_compiled(compiled, input)?;
            input.output.validate_equality("distinct")?;
            let fingerprint =
                fingerprint_unary_stage(b"pipeline/distinct/v1", builder.fingerprint(input.id)?);
            let id = builder.push(LogicalNode::Distinct { input: input.id }, fingerprint)?;
            Ok(Compiled { id, ..input })
        }
        PipelineNode::Slice {
            input,
            offset,
            limit,
        } => {
            let input = require_compiled(compiled, input)?;
            let fingerprint =
                fingerprint_slice_stage(builder.fingerprint(input.id)?, *offset, *limit);
            let id = builder.push(
                LogicalNode::Slice {
                    input: input.id,
                    offset: *offset,
                    limit: *limit,
                },
                fingerprint,
            )?;
            Ok(Compiled { id, ..input })
        }
        PipelineNode::Join {
            left,
            right,
            kind,
            condition,
        } => compile_join(
            compiled,
            builder,
            limits,
            JoinCompile {
                left,
                right,
                kind: *kind,
                condition: condition.as_ref(),
            },
        ),
        PipelineNode::Set {
            left,
            right,
            operator,
        } => compile_set(compiled, builder, left, right, *operator),
    }
}

fn compile_projection(
    projection: &ProjectionSpec,
    context: &BindContext<'_>,
    limits: PipelineLimits,
) -> Result<LogicalProjection> {
    validate_type(&projection.ty, DefinitionLimits::default())?;
    validate_type_universe([&projection.ty])?;
    let mut components = projection
        .expressions
        .iter()
        .cloned()
        .enumerate()
        .map(|(index, expression)| {
            let name = projection.names.get(index).cloned();
            (name, expression)
        })
        .collect::<Vec<_>>();

    if projection.kind == ProjectionKind::Record {
        if projection.names.len() != projection.expressions.len() {
            return Err(Diagnostic::error(
                "PIPELINE-PROJECT-001",
                "record projection field names and expressions have different widths",
            ));
        }
        components.sort_by(|left, right| left.0.as_deref().cmp(&right.0.as_deref()));
        if components
            .windows(2)
            .any(|pair| pair[0].0.as_deref() == pair[1].0.as_deref())
        {
            return Err(Diagnostic::error(
                "PIPELINE-PROJECT-002",
                "record projection contains duplicate field names",
            ));
        }
    } else if !projection.names.is_empty() {
        return Err(Diagnostic::error(
            "PIPELINE-PROJECT-003",
            "only named-record projections may carry field names",
        ));
    }

    let mut names = Vec::with_capacity(components.len());
    let mut expressions = Vec::with_capacity(components.len());
    for (name, expression) in components {
        if let Some(name) = name {
            names.push(name);
        }
        expressions.push(LogicalExpr::from_prepared(prepare_expression(
            &expression,
            context,
            limits.expression,
        )?));
    }

    match projection.kind {
        ProjectionKind::Value => {
            if expressions.len() != 1 || expressions[0].type_def() != &projection.ty {
                return Err(Diagnostic::error(
                    "PIPELINE-PROJECT-004",
                    "scalar projection output type does not match its expression",
                ));
            }
        }
        ProjectionKind::Tuple => match projection.ty.shape() {
            TypeShape::Tuple(elements)
                if elements.len() == expressions.len()
                    && elements
                        .iter()
                        .zip(&expressions)
                        .all(|(expected, expression)| expected == expression.type_def()) => {}
            _ => {
                return Err(Diagnostic::error(
                    "PIPELINE-PROJECT-005",
                    "tuple projection output type does not match its component expressions",
                ));
            }
        },
        ProjectionKind::Record => match projection.ty.shape() {
            TypeShape::Record(fields)
                if fields.len() == expressions.len()
                    && fields.iter().zip(names.iter().zip(&expressions)).all(
                        |(field, (name, expression))| {
                            field.name() == name.as_ref() && field.ty() == expression.type_def()
                        },
                    ) => {}
            _ => {
                return Err(Diagnostic::error(
                    "PIPELINE-PROJECT-006",
                    "record projection output type does not match its named expressions",
                ));
            }
        },
    }

    Ok(LogicalProjection::new(
        projection.kind,
        expressions.into_boxed_slice(),
        names.into_boxed_slice(),
        projection.ty.clone(),
    ))
}

fn compile_existential(
    builder: &mut PlanBuilder,
    limits: PipelineLimits,
    outer_scopes: &[Scope],
    exists: &ExistentialRef,
) -> Result<PreparedExistential> {
    let subquery = compile_pipeline(exists.subquery(), builder, limits, outer_scopes)?;
    Ok(PreparedExistential {
        subquery: subquery.id,
        fingerprint: builder.fingerprint(subquery.id)?,
    })
}

fn compile_source(source: &super::ModelSource, builder: &mut PlanBuilder) -> Result<Compiled> {
    let model = source.resolve()?;
    if let Some(alias) = source.alias() {
        validate_alias(alias)?;
    }
    let id = builder.push(
        LogicalNode::Source {
            model: Box::new(model.clone()),
            alias: source.alias().cloned(),
        },
        fingerprint_source(model),
    )?;
    Ok(Compiled {
        id,
        scopes: vec![Scope {
            model,
            alias: source.alias().cloned(),
            nullable: false,
            identity_unique: true,
        }],
        output: Output::Model(model),
    })
}

struct JoinCompile<'a> {
    left: &'a Arc<PipelineNode>,
    right: &'a Arc<PipelineNode>,
    kind: JoinKind,
    condition: Option<&'a crate::expr::ExpressionSpec>,
}

fn compile_join(
    compiled: &HashMap<*const PipelineNode, Compiled>,
    builder: &mut PlanBuilder,
    limits: PipelineLimits,
    join: JoinCompile<'_>,
) -> Result<Compiled> {
    let left = require_compiled(compiled, join.left)?;
    let right = require_compiled(compiled, join.right)?;
    let condition_scopes = combine_scopes(&left.scopes, &right.scopes)?;
    let condition = match (join.kind, join.condition) {
        (JoinKind::Inner | JoinKind::Left | JoinKind::Right | JoinKind::Full, Some(condition)) => {
            let context = bind_context(&condition_scopes);
            Some(LogicalExpr::from_prepared(prepare_expression(
                condition,
                &context,
                limits.expression,
            )?))
        }
        (JoinKind::Cross, None) => None,
        (JoinKind::Inner | JoinKind::Left | JoinKind::Right | JoinKind::Full, None) => {
            return Err(Diagnostic::error(
                "PIPELINE-PLAN-006",
                "conditional join is missing its truth-valued condition",
            ));
        }
        (JoinKind::Cross, Some(_)) => {
            return Err(Diagnostic::error(
                "PIPELINE-PLAN-007",
                "cross join cannot carry a join condition",
            ));
        }
    };

    let fingerprint = fingerprint_join_stage(
        builder.fingerprint(left.id)?,
        builder.fingerprint(right.id)?,
        join.kind,
        condition.as_ref(),
    );
    let id = builder.push(
        LogicalNode::Join {
            left: left.id,
            right: right.id,
            kind: join.kind,
            condition: condition.map(Box::new),
        },
        fingerprint,
    )?;
    let left_scope_count = left.scopes.len();
    let scopes = lift_join_scopes(condition_scopes, left_scope_count, join.kind);
    let output = match join.kind {
        JoinKind::Inner | JoinKind::Cross => {
            Output::Product(vec![left.output, right.output].into_boxed_slice())
        }
        JoinKind::Left => {
            Output::Product(vec![left.output, right.output.nullable()].into_boxed_slice())
        }
        JoinKind::Right => {
            Output::Product(vec![left.output.nullable(), right.output].into_boxed_slice())
        }
        JoinKind::Full => Output::Product(
            vec![left.output.nullable(), right.output.nullable()].into_boxed_slice(),
        ),
    };
    Ok(Compiled { id, scopes, output })
}

fn compile_set(
    compiled: &HashMap<*const PipelineNode, Compiled>,
    builder: &mut PlanBuilder,
    left_node: &Arc<PipelineNode>,
    right_node: &Arc<PipelineNode>,
    operator: SetOperator,
) -> Result<Compiled> {
    let left = require_compiled(compiled, left_node)?;
    let right = require_compiled(compiled, right_node)?;
    if !left.output.semantically_matches(&right.output) {
        return Err(Diagnostic::error(
            "PIPELINE-TYPE-003",
            "set operations require identical semantic output shapes",
        ));
    }
    if operator.requires_equality() {
        left.output.validate_equality("set operation")?;
    }

    let fingerprint = fingerprint_set_stage(
        builder.fingerprint(left.id)?,
        builder.fingerprint(right.id)?,
        operator,
    );
    let id = builder.push(
        LogicalNode::Set {
            left: left.id,
            right: right.id,
            operator,
        },
        fingerprint,
    )?;
    let scopes = compatible_set_scopes(&left.scopes, &right.scopes, operator);
    Ok(Compiled {
        id,
        scopes,
        output: left.output,
    })
}

fn require_compiled(
    compiled: &HashMap<*const PipelineNode, Compiled>,
    input: &Arc<PipelineNode>,
) -> Result<Compiled> {
    compiled.get(&Arc::as_ptr(input)).cloned().ok_or_else(|| {
        Diagnostic::error(
            "PIPELINE-PLAN-004",
            "logical pipeline node is missing a compiled semantic input",
        )
    })
}

fn combine_scopes(left: &[Scope], right: &[Scope]) -> Result<Vec<Scope>> {
    let mut scopes = Vec::with_capacity(left.len() + right.len());
    scopes.extend_from_slice(left);
    scopes.extend_from_slice(right);

    let mut aliases = HashSet::new();
    for scope in &scopes {
        if let Some(alias) = &scope.alias
            && !aliases.insert(alias.as_ref())
        {
            return Err(Diagnostic::error(
                "PIPELINE-SCOPE-002",
                format!("pipeline source alias `{alias}` appears more than once"),
            ));
        }
    }

    for (index, scope) in scopes.iter().enumerate() {
        let duplicate_model = scopes.iter().enumerate().any(|(other_index, other)| {
            index != other_index && other.model.key() == scope.model.key()
        });
        if duplicate_model && scope.alias.is_none() {
            return Err(Diagnostic::error(
                "PIPELINE-SCOPE-001",
                format!(
                    "model `{}` appears more than once; every occurrence must have an explicit source alias",
                    scope.model.key().as_str()
                ),
            ));
        }
    }

    Ok(scopes)
}

fn lift_join_scopes(mut scopes: Vec<Scope>, left_count: usize, kind: JoinKind) -> Vec<Scope> {
    for scope in &mut scopes {
        scope.identity_unique = false;
    }
    match kind {
        JoinKind::Inner | JoinKind::Cross => {}
        JoinKind::Left => {
            for scope in scopes.iter_mut().skip(left_count) {
                scope.nullable = true;
            }
        }
        JoinKind::Right => {
            for scope in scopes.iter_mut().take(left_count) {
                scope.nullable = true;
            }
        }
        JoinKind::Full => {
            for scope in &mut scopes {
                scope.nullable = true;
            }
        }
    }
    scopes
}

fn compatible_set_scopes(left: &[Scope], right: &[Scope], operator: SetOperator) -> Vec<Scope> {
    match (left, right) {
        ([left], [right]) if left.model.fingerprint() == right.model.fingerprint() => {
            let identity_unique = match operator {
                SetOperator::Intersect => left.identity_unique || right.identity_unique,
                SetOperator::Except => left.identity_unique,
                SetOperator::Union | SetOperator::UnionAll => false,
            };
            vec![Scope {
                model: left.model,
                alias: None,
                nullable: left.nullable || right.nullable,
                identity_unique,
            }]
        }
        _ => Vec::new(),
    }
}

fn validate_alias(alias: &str) -> Result<()> {
    if alias.is_empty() || alias.len() > 64 {
        return Err(Diagnostic::error(
            "PIPELINE-SCOPE-003",
            "pipeline source alias must contain between 1 and 64 ASCII characters",
        ));
    }
    let mut chars = alias.chars();
    let first = chars.next().expect("non-empty alias was checked above");
    if !(first.is_ascii_alphabetic() || first == '_')
        || chars.any(|character| !(character.is_ascii_alphanumeric() || character == '_'))
    {
        return Err(Diagnostic::error(
            "PIPELINE-SCOPE-003",
            format!("pipeline source alias `{alias}` is not a valid ASCII identifier"),
        ));
    }
    Ok(())
}

fn bind_context(scopes: &[Scope]) -> BindContext<'static> {
    scopes.iter().fold(BindContext::new(), |context, scope| {
        match (&scope.alias, scope.nullable) {
            (Some(alias), true) => context.with_nullable_alias(Arc::clone(alias), scope.model),
            (Some(alias), false) => context.with_alias(Arc::clone(alias), scope.model),
            (None, true) => context.with_nullable_model(scope.model),
            (None, false) => context.with_model(scope.model),
        }
    })
}
