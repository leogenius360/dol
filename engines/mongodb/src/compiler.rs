//! Exact offline lowering of the conservative MongoDB read slice.

use dol_core::diagnostic::{Diagnostic, Result};
use dol_core::expr::Parameters;
use dol_core::model::{ModelDef, ModelKey, Presence};
use dol_core::plan::{
    LogicalBinaryOp, LogicalExpr, LogicalExprNodeKind, LogicalNode, LogicalPlan, LogicalUnaryOp,
    PlanId, PlanOutput, ProjectionKind,
};
use dol_core::types::{Nullability, ScalarRepr, TypeDef, TypeShape};
use dol_core::value::{Datum, validate_datum};
use mongodb::bson::{Bson, Decimal128, Document, doc};

use crate::codec::{encode_datum, ensure_supported_type};
use crate::compiled::{BsonOutputField, CompiledAggregation};
use crate::mapping::MongodbCatalog;

const STATE_MISSING: i32 = 0;
const STATE_NULL: i32 = 1;
const STATE_VALUE: i32 = 2;
const TRUTH_FALSE: i32 = 0;
const TRUTH_TRUE: i32 = 1;
const TRUTH_UNKNOWN: i32 = 2;

/// Offline compiler for MongoDB's exact source/filter/project/slice subset.
#[derive(Debug, Clone, Copy)]
pub struct MongodbCompiler<'a> {
    catalog: &'a MongodbCatalog,
}

impl<'a> MongodbCompiler<'a> {
    /// Creates a compiler borrowing immutable physical mappings.
    #[must_use]
    pub const fn new(catalog: &'a MongodbCatalog) -> Self {
        Self { catalog }
    }

    pub(crate) fn supports_node(&self, node: &LogicalNode) -> bool {
        match node {
            LogicalNode::Source { model, .. } => {
                self.catalog.collection(model).is_ok()
                    && model
                        .fields()
                        .iter()
                        .all(|field| ensure_supported_type(field.ty()).is_ok())
            }
            LogicalNode::Filter { condition, .. } => {
                matches!(
                    condition.type_def().shape(),
                    TypeShape::Scalar(ScalarRepr::Truth)
                ) && expression_support(condition).is_some_and(|always_value| always_value)
            }
            LogicalNode::Project { projection, .. } => projection
                .expressions()
                .iter()
                .all(|expression| expression_support(expression).is_some()),
            LogicalNode::Slice { offset, limit, .. } => {
                i64::try_from(*offset).is_ok()
                    && limit.is_none_or(|value| i64::try_from(value).is_ok())
            }
            _ => false,
        }
    }

    /// Compiles a validated logical plan without opening a network connection.
    pub fn compile(
        &self,
        plan: &LogicalPlan,
        parameters: &Parameters,
    ) -> Result<CompiledAggregation> {
        // Iterate the topological plan rather than recursively descending it. This
        // keeps compilation stack use bounded even for adversarially deep plans.
        let mut relations = Vec::<Relation>::with_capacity(plan.nodes().len());
        for (index, node) in plan.nodes().iter().enumerate() {
            let relation = match node {
                LogicalNode::Source { model, .. } => self.compile_source(model, index)?,
                LogicalNode::Filter { input, condition } => {
                    let input = relation(&relations, *input)?.clone();
                    self.compile_filter(input, condition, parameters)?
                }
                LogicalNode::Project { input, projection } => {
                    let input = relation(&relations, *input)?.clone();
                    self.compile_project(input, projection, parameters, index)?
                }
                LogicalNode::Slice {
                    input,
                    offset,
                    limit,
                } => {
                    let input = relation(&relations, *input)?.clone();
                    self.compile_slice(input, *offset, *limit)?
                }
                _ => {
                    return Err(Diagnostic::error(
                        "MONGODB-COMPILE-001",
                        "MongoDB exact lowering currently supports source/filter/project/slice only",
                    ));
                }
            };
            relations.push(relation);
        }

        let relation = relation(&relations, plan.root())?.clone();
        if &relation.output != plan.output() {
            return Err(Diagnostic::error(
                "MONGODB-COMPILE-002",
                "compiled MongoDB output differs from the logical plan output",
            ));
        }
        Ok(CompiledAggregation::new(
            (relation.database, relation.collection),
            relation.validation_pipeline,
            relation.pipeline,
            relation.output_fields,
            relation.output,
        ))
    }

    fn compile_source(&self, model: &ModelDef, stage: usize) -> Result<Relation> {
        let mapping = self.catalog.collection(model)?;
        let mut projection = Document::new();
        projection.insert("_id", Bson::Int32(0));
        let mut fields = Vec::with_capacity(model.fields().len());
        let mut output_fields = Vec::with_capacity(model.fields().len());
        let mut validity = Vec::with_capacity(model.fields().len());

        for (index, field) in model.fields().iter().enumerate() {
            ensure_supported_type(field.ty())?;
            let physical = mapping.field_mapping(field.key().as_str()).ok_or_else(|| {
                Diagnostic::error(
                    "MONGODB-MAP-002",
                    format!("field `{}` has no BSON mapping", field.key().as_str()),
                )
            })?;
            let state_field = format!("__dol_s{stage}_f{index}_state");
            let value_field = format!("__dol_s{stage}_f{index}_value");
            let value = field_path(physical.name());
            let state = source_state(value.clone());
            projection.insert(state_field.clone(), state.clone());
            projection.insert(value_field.clone(), value.clone());
            validity.push(source_field_valid(
                field.presence(),
                field.ty(),
                state.clone(),
                value,
            )?);
            let column = ColumnRef {
                state_field: state_field.clone(),
                value_field: value_field.clone(),
                always_value: field.presence() == Presence::Required
                    && field.ty().nullability() == Nullability::NonNull,
            };
            fields.push(column.clone());
            output_fields.push(BsonOutputField::new(
                state_field,
                value_field,
                field.ty().clone(),
            ));
        }

        let valid = and(validity);
        let validation_pipeline = vec![
            doc! { "$match": { "$expr": op("$not", vec![valid]) } },
            doc! { "$limit": 1_i64 },
            doc! { "$project": { "_id": 1_i32 } },
        ];
        Ok(Relation {
            database: mapping.database().to_owned(),
            collection: mapping.collection().to_owned(),
            validation_pipeline,
            pipeline: vec![doc! { "$project": projection }],
            scopes: vec![ScopeLayout {
                model_key: model.key().clone(),
                fields,
            }],
            output_fields,
            output: PlanOutput::Model(Box::new(model.clone())),
        })
    }

    fn compile_filter(
        &self,
        mut relation: Relation,
        condition: &LogicalExpr,
        parameters: &Parameters,
    ) -> Result<Relation> {
        ensure_truth_type(condition.type_def())?;
        let truth = compile_expr(condition, &relation, parameters)?;
        if !truth.always_value {
            return Err(Diagnostic::error(
                "MONGODB-COMPILE-003",
                "filter condition may itself be Missing/Null and cannot be lowered exactly",
            ));
        }
        relation.pipeline.push(doc! {
            "$match": {
                "$expr": and(vec![
                    eq(truth.state, literal(STATE_VALUE)),
                    eq(truth.value, literal(TRUTH_TRUE)),
                ])
            }
        });
        Ok(relation)
    }

    fn compile_project(
        &self,
        mut relation: Relation,
        projection: &dol_core::plan::LogicalProjection,
        parameters: &Parameters,
        stage: usize,
    ) -> Result<Relation> {
        let mut document = Document::new();
        document.insert("_id", Bson::Int32(0));
        let mut output_fields = Vec::with_capacity(projection.expressions().len());
        for (index, expression) in projection.expressions().iter().enumerate() {
            ensure_supported_type(expression.type_def())?;
            let datum = compile_expr(expression, &relation, parameters)?;
            let state_field = format!("__dol_p{stage}_f{index}_state");
            let value_field = format!("__dol_p{stage}_f{index}_value");
            document.insert(state_field.clone(), datum.state);
            document.insert(value_field.clone(), datum.value);
            output_fields.push(BsonOutputField::new(
                state_field,
                value_field,
                expression.type_def().clone(),
            ));
        }
        let expected = match projection.kind() {
            ProjectionKind::Value => 1,
            ProjectionKind::Tuple | ProjectionKind::Record => projection.expressions().len(),
        };
        if output_fields.len() != expected {
            return Err(Diagnostic::error(
                "MONGODB-COMPILE-004",
                "projection arity changed during MongoDB lowering",
            ));
        }
        relation.pipeline.push(doc! { "$project": document });
        relation.scopes.clear();
        relation.output_fields = output_fields;
        relation.output = PlanOutput::Value(projection.type_def().clone());
        Ok(relation)
    }

    fn compile_slice(
        &self,
        mut relation: Relation,
        offset: u64,
        limit: Option<u64>,
    ) -> Result<Relation> {
        if offset != 0 {
            relation.pipeline.push(doc! {
                "$skip": i64::try_from(offset).map_err(|_| slice_range_error())?
            });
        }
        if let Some(limit) = limit {
            relation.pipeline.push(doc! {
                "$limit": i64::try_from(limit).map_err(|_| slice_range_error())?
            });
        }
        Ok(relation)
    }
}

fn expression_support(expression: &LogicalExpr) -> Option<bool> {
    let view = expression.engine_view();
    if view.outer_scope_count() != 0 {
        return None;
    }
    let mut always_values = Vec::with_capacity(view.nodes().len());
    for node in view.nodes() {
        if ensure_supported_type(node.type_def()).is_err()
            && !matches!(
                node.type_def().shape(),
                TypeShape::Scalar(ScalarRepr::Truth)
            )
        {
            return None;
        }
        let always_value = match node.kind() {
            LogicalExprNodeKind::Field {
                allow_null_extension,
                ..
            } => {
                if *allow_null_extension {
                    return None;
                }
                false
            }
            LogicalExprNodeKind::Literal(datum) => matches!(**datum, Datum::Value(_)),
            LogicalExprNodeKind::Parameter(_) => false,
            LogicalExprNodeKind::Unary { op, input } => match op {
                LogicalUnaryOp::IsNull | LogicalUnaryOp::IsMissing | LogicalUnaryOp::IsPresent => {
                    true
                }
                LogicalUnaryOp::NullableLift => *always_values.get(*input)?,
                LogicalUnaryOp::Not => {
                    if !*always_values.get(*input)? {
                        return None;
                    }
                    true
                }
                _ => return None,
            },
            LogicalExprNodeKind::Binary { op, left, right } => {
                let left = *always_values.get(*left)?;
                let right = *always_values.get(*right)?;
                match op {
                    LogicalBinaryOp::Eq
                    | LogicalBinaryOp::Ne
                    | LogicalBinaryOp::Lt
                    | LogicalBinaryOp::Le
                    | LogicalBinaryOp::Gt
                    | LogicalBinaryOp::Ge
                    | LogicalBinaryOp::IsDistinctFrom
                    | LogicalBinaryOp::IsNotDistinctFrom => true,
                    LogicalBinaryOp::And | LogicalBinaryOp::Or => {
                        if !(left && right) {
                            return None;
                        }
                        true
                    }
                    LogicalBinaryOp::Coalesce => left || right,
                    _ => return None,
                }
            }
            LogicalExprNodeKind::Membership {
                input, candidates, ..
            } => {
                always_values.get(*input)?;
                for candidate in candidates {
                    always_values.get(*candidate)?;
                }
                true
            }
            LogicalExprNodeKind::Conditional {
                condition,
                when_true,
                when_false,
            } => {
                if !*always_values.get(*condition)? {
                    return None;
                }
                *always_values.get(*when_true)? && *always_values.get(*when_false)?
            }
            LogicalExprNodeKind::FunctionCall { .. } | LogicalExprNodeKind::Exists { .. } => {
                return None;
            }
            _ => return None,
        };
        always_values.push(always_value);
    }
    always_values.get(view.root()).copied()
}

#[derive(Debug, Clone)]
struct ColumnRef {
    state_field: String,
    value_field: String,
    always_value: bool,
}

#[derive(Debug, Clone)]
struct ScopeLayout {
    model_key: ModelKey,
    fields: Vec<ColumnRef>,
}

#[derive(Debug, Clone)]
struct Relation {
    database: String,
    collection: String,
    validation_pipeline: Vec<Document>,
    pipeline: Vec<Document>,
    scopes: Vec<ScopeLayout>,
    output_fields: Vec<BsonOutputField>,
    output: PlanOutput,
}

#[derive(Debug, Clone)]
struct BsonDatum {
    state: Bson,
    value: Bson,
    ty: TypeDef,
    always_value: bool,
}

fn compile_expr(
    expression: &LogicalExpr,
    relation: &Relation,
    parameters: &Parameters,
) -> Result<BsonDatum> {
    let view = expression.engine_view();
    if view.outer_scope_count() != 0 {
        return Err(Diagnostic::error(
            "MONGODB-COMPILE-005",
            "correlated MongoDB expression lowering is not implemented",
        ));
    }
    let mut values = Vec::<BsonDatum>::with_capacity(view.nodes().len());
    for node in view.nodes() {
        let datum = match node.kind() {
            LogicalExprNodeKind::Field {
                scope,
                slot,
                allow_null_extension,
            } => {
                if *allow_null_extension {
                    return Err(Diagnostic::error(
                        "MONGODB-COMPILE-006",
                        "outer-join null extension is outside the MongoDB read slice",
                    ));
                }
                let expected = view.scopes().get(*scope).ok_or_else(invalid_expression)?;
                let layout = relation.scopes.get(*scope).ok_or_else(invalid_expression)?;
                if expected != &layout.model_key {
                    return Err(Diagnostic::error(
                        "MONGODB-COMPILE-007",
                        "expression scope does not match the MongoDB source scope",
                    ));
                }
                let column = layout.fields.get(*slot).ok_or_else(invalid_expression)?;
                BsonDatum {
                    state: field_path(&column.state_field),
                    value: field_path(&column.value_field),
                    ty: node.type_def().clone(),
                    always_value: column.always_value,
                }
            }
            LogicalExprNodeKind::Literal(datum) => bound_datum(node.type_def(), (**datum).clone())?,
            LogicalExprNodeKind::Parameter(name) => {
                let parameter = parameters.get(name).ok_or_else(|| {
                    Diagnostic::error(
                        "MONGODB-PARAM-001",
                        format!("parameter `{name}` is not bound"),
                    )
                })?;
                if parameter.type_def() != node.type_def() {
                    return Err(Diagnostic::error(
                        "MONGODB-PARAM-002",
                        format!("parameter `{name}` has a different exact semantic type"),
                    ));
                }
                bound_datum(node.type_def(), parameter.datum().clone())?
            }
            LogicalExprNodeKind::Unary { op: unary, input } => {
                compile_unary(*unary, value(&values, *input)?, node.type_def())?
            }
            LogicalExprNodeKind::Binary {
                op: binary,
                left,
                right,
            } => compile_binary(
                *binary,
                value(&values, *left)?,
                value(&values, *right)?,
                node.type_def(),
            )?,
            LogicalExprNodeKind::Membership {
                input,
                candidates,
                negate,
            } => {
                let input = value(&values, *input)?;
                let candidates = candidates
                    .iter()
                    .map(|index| value(&values, *index))
                    .collect::<Result<Vec<_>>>()?;
                compile_membership(input, &candidates, *negate)?
            }
            LogicalExprNodeKind::Conditional {
                condition,
                when_true,
                when_false,
            } => compile_conditional(
                value(&values, *condition)?,
                value(&values, *when_true)?,
                value(&values, *when_false)?,
                node.type_def(),
            )?,
            LogicalExprNodeKind::FunctionCall { .. } => {
                return Err(Diagnostic::error(
                    "MONGODB-COMPILE-008",
                    "semantic function lowering is not implemented by MongoDB",
                ));
            }
            LogicalExprNodeKind::Exists { .. } => {
                return Err(Diagnostic::error(
                    "MONGODB-COMPILE-009",
                    "existential MongoDB lowering is not implemented",
                ));
            }
            _ => return Err(newer_expression()),
        };
        values.push(datum);
    }
    values
        .get(view.root())
        .cloned()
        .ok_or_else(invalid_expression)
}

fn bound_datum(ty: &TypeDef, datum: Datum) -> Result<BsonDatum> {
    ensure_supported_type(ty)?;
    validate_datum(ty, Presence::Optional, &datum).map_err(|error| {
        Diagnostic::error(
            "MONGODB-PARAM-003",
            format!("invalid typed parameter: {}", error.message()),
        )
    })?;
    let state = match &datum {
        Datum::Missing => STATE_MISSING,
        Datum::Null => STATE_NULL,
        Datum::Value(_) => STATE_VALUE,
    };
    Ok(BsonDatum {
        state: literal(state),
        value: literal_bson(encode_datum(ty, &datum)?),
        ty: ty.clone(),
        always_value: state == STATE_VALUE,
    })
}

fn compile_unary(opcode: LogicalUnaryOp, input: BsonDatum, output: &TypeDef) -> Result<BsonDatum> {
    match opcode {
        LogicalUnaryOp::IsNull => Ok(truth_datum(cond(
            eq(input.state, literal(STATE_NULL)),
            literal(TRUTH_TRUE),
            literal(TRUTH_FALSE),
        ))),
        LogicalUnaryOp::IsMissing => Ok(truth_datum(cond(
            eq(input.state, literal(STATE_MISSING)),
            literal(TRUTH_TRUE),
            literal(TRUTH_FALSE),
        ))),
        LogicalUnaryOp::IsPresent => Ok(truth_datum(cond(
            eq(input.state, literal(STATE_MISSING)),
            literal(TRUTH_FALSE),
            literal(TRUTH_TRUE),
        ))),
        LogicalUnaryOp::NullableLift => Ok(BsonDatum {
            ty: output.clone(),
            ..input
        }),
        LogicalUnaryOp::Not => {
            require_total_truth(&input)?;
            Ok(truth_datum(cond(
                eq(input.value.clone(), literal(TRUTH_TRUE)),
                literal(TRUTH_FALSE),
                cond(
                    eq(input.value, literal(TRUTH_FALSE)),
                    literal(TRUTH_TRUE),
                    literal(TRUTH_UNKNOWN),
                ),
            )))
        }
        LogicalUnaryOp::LosslessCast | LogicalUnaryOp::Negate | LogicalUnaryOp::Abs => {
            Err(Diagnostic::error(
                "MONGODB-COMPILE-010",
                "checked casts and unary arithmetic are not lowered by MongoDB",
            ))
        }
        _ => Err(newer_expression()),
    }
}

fn compile_binary(
    opcode: LogicalBinaryOp,
    left: BsonDatum,
    right: BsonDatum,
    output: &TypeDef,
) -> Result<BsonDatum> {
    match opcode {
        LogicalBinaryOp::Eq | LogicalBinaryOp::Ne => {
            let equal = value_equal(&left, &right)?;
            let comparison = if opcode == LogicalBinaryOp::Eq {
                equal
            } else {
                op("$not", vec![equal])
            };
            Ok(comparison_truth(&left, &right, comparison))
        }
        LogicalBinaryOp::Lt | LogicalBinaryOp::Le | LogicalBinaryOp::Gt | LogicalBinaryOp::Ge => {
            let comparison = value_order(opcode, &left, &right)?;
            Ok(comparison_truth(&left, &right, comparison))
        }
        LogicalBinaryOp::IsDistinctFrom | LogicalBinaryOp::IsNotDistinctFrom => {
            let same_state = eq(left.state.clone(), right.state.clone());
            let both_value = and(vec![
                eq(left.state.clone(), literal(STATE_VALUE)),
                eq(right.state.clone(), literal(STATE_VALUE)),
            ]);
            let equal = and(vec![
                same_state,
                cond(both_value, value_equal(&left, &right)?, literal(true)),
            ]);
            let result = if opcode == LogicalBinaryOp::IsNotDistinctFrom {
                equal
            } else {
                op("$not", vec![equal])
            };
            Ok(truth_datum(cond(
                result,
                literal(TRUTH_TRUE),
                literal(TRUTH_FALSE),
            )))
        }
        LogicalBinaryOp::And | LogicalBinaryOp::Or => {
            require_total_truth(&left)?;
            require_total_truth(&right)?;
            let result = if opcode == LogicalBinaryOp::And {
                cond(
                    or(vec![
                        eq(left.value.clone(), literal(TRUTH_FALSE)),
                        eq(right.value.clone(), literal(TRUTH_FALSE)),
                    ]),
                    literal(TRUTH_FALSE),
                    cond(
                        and(vec![
                            eq(left.value, literal(TRUTH_TRUE)),
                            eq(right.value, literal(TRUTH_TRUE)),
                        ]),
                        literal(TRUTH_TRUE),
                        literal(TRUTH_UNKNOWN),
                    ),
                )
            } else {
                cond(
                    or(vec![
                        eq(left.value.clone(), literal(TRUTH_TRUE)),
                        eq(right.value.clone(), literal(TRUTH_TRUE)),
                    ]),
                    literal(TRUTH_TRUE),
                    cond(
                        and(vec![
                            eq(left.value, literal(TRUTH_FALSE)),
                            eq(right.value, literal(TRUTH_FALSE)),
                        ]),
                        literal(TRUTH_FALSE),
                        literal(TRUTH_UNKNOWN),
                    ),
                )
            };
            Ok(truth_datum(result))
        }
        LogicalBinaryOp::Coalesce => {
            let choose_left = eq(left.state.clone(), literal(STATE_VALUE));
            Ok(BsonDatum {
                state: cond(choose_left.clone(), left.state, right.state),
                value: cond(choose_left, left.value, right.value),
                ty: output.clone(),
                always_value: left.always_value || right.always_value,
            })
        }
        LogicalBinaryOp::Add
        | LogicalBinaryOp::Sub
        | LogicalBinaryOp::Mul
        | LogicalBinaryOp::Div
        | LogicalBinaryOp::Rem => Err(Diagnostic::error(
            "MONGODB-COMPILE-011",
            "checked arithmetic cannot be represented exactly by the MongoDB read slice",
        )),
        _ => Err(newer_expression()),
    }
}

fn compile_membership(
    input: BsonDatum,
    candidates: &[BsonDatum],
    negate: bool,
) -> Result<BsonDatum> {
    let comparisons = candidates
        .iter()
        .map(|candidate| {
            value_equal(&input, candidate).map(|equal| comparison_truth(&input, candidate, equal))
        })
        .collect::<Result<Vec<_>>>()?;
    if comparisons.is_empty() {
        return Ok(truth_datum(literal(if negate {
            TRUTH_TRUE
        } else {
            TRUTH_FALSE
        })));
    }
    let saw_true = or(comparisons
        .iter()
        .map(|value| eq(value.value.clone(), literal(TRUTH_TRUE)))
        .collect());
    let saw_unknown = or(comparisons
        .iter()
        .map(|value| eq(value.value.clone(), literal(TRUTH_UNKNOWN)))
        .collect());
    Ok(truth_datum(cond(
        saw_true,
        literal(if negate { TRUTH_FALSE } else { TRUTH_TRUE }),
        cond(
            saw_unknown,
            literal(TRUTH_UNKNOWN),
            literal(if negate { TRUTH_TRUE } else { TRUTH_FALSE }),
        ),
    )))
}

fn compile_conditional(
    condition: BsonDatum,
    when_true: BsonDatum,
    when_false: BsonDatum,
    output: &TypeDef,
) -> Result<BsonDatum> {
    require_total_truth(&condition)?;
    let predicate = eq(condition.value, literal(TRUTH_TRUE));
    Ok(BsonDatum {
        state: cond(predicate.clone(), when_true.state, when_false.state),
        value: cond(predicate, when_true.value, when_false.value),
        ty: output.clone(),
        always_value: when_true.always_value && when_false.always_value,
    })
}

fn comparison_truth(left: &BsonDatum, right: &BsonDatum, comparison: Bson) -> BsonDatum {
    truth_datum(cond(
        or(vec![
            op("$ne", vec![left.state.clone(), literal(STATE_VALUE)]),
            op("$ne", vec![right.state.clone(), literal(STATE_VALUE)]),
        ]),
        literal(TRUTH_UNKNOWN),
        cond(comparison, literal(TRUTH_TRUE), literal(TRUTH_FALSE)),
    ))
}

fn value_equal(left: &BsonDatum, right: &BsonDatum) -> Result<Bson> {
    let repr = same_scalar_repr(&left.ty, &right.ty)?;
    let equal = eq(left.value.clone(), right.value.clone());
    Ok(match repr {
        ScalarRepr::Float32 | ScalarRepr::Float64 => and(vec![
            not_nan(left.value.clone()),
            not_nan(right.value.clone()),
            equal,
        ]),
        _ => equal,
    })
}

fn value_order(opcode: LogicalBinaryOp, left: &BsonDatum, right: &BsonDatum) -> Result<Bson> {
    let repr = same_scalar_repr(&left.ty, &right.ty)?;
    let operator = match opcode {
        LogicalBinaryOp::Lt => "$lt",
        LogicalBinaryOp::Le => "$lte",
        LogicalBinaryOp::Gt => "$gt",
        LogicalBinaryOp::Ge => "$gte",
        _ => return Err(newer_expression()),
    };
    let (left_value, right_value) = match repr {
        ScalarRepr::Truth => (
            truth_order_rank(left.value.clone()),
            truth_order_rank(right.value.clone()),
        ),
        ScalarRepr::Bool
        | ScalarRepr::Int { .. }
        | ScalarRepr::UInt { .. }
        | ScalarRepr::Float32
        | ScalarRepr::Float64
        | ScalarRepr::Char
        | ScalarRepr::String => (left.value.clone(), right.value.clone()),
        _ => {
            return Err(Diagnostic::error(
                "MONGODB-COMPILE-012",
                "this BSON scalar family has no proven exact MongoDB ordering lowering",
            ));
        }
    };
    let ordered = op(operator, vec![left_value, right_value]);
    Ok(match repr {
        ScalarRepr::Float32 | ScalarRepr::Float64 => and(vec![
            not_nan(left.value.clone()),
            not_nan(right.value.clone()),
            ordered,
        ]),
        _ => ordered,
    })
}

fn same_scalar_repr(left: &TypeDef, right: &TypeDef) -> Result<ScalarRepr> {
    ensure_supported_type(left)?;
    ensure_supported_type(right)?;
    match (left.shape(), right.shape()) {
        (TypeShape::Scalar(left), TypeShape::Scalar(right)) if left == right => Ok(*left),
        _ => Err(Diagnostic::error(
            "MONGODB-COMPILE-013",
            "MongoDB comparison operands do not share one scalar representation",
        )),
    }
}

fn require_total_truth(value: &BsonDatum) -> Result<()> {
    ensure_truth_type(&value.ty)?;
    if value.always_value {
        Ok(())
    } else {
        Err(Diagnostic::error(
            "MONGODB-COMPILE-014",
            "truth operation could receive Missing/Null and must remain local",
        ))
    }
}

fn ensure_truth_type(ty: &TypeDef) -> Result<()> {
    if matches!(ty.shape(), TypeShape::Scalar(ScalarRepr::Truth)) {
        Ok(())
    } else {
        Err(Diagnostic::error(
            "MONGODB-COMPILE-015",
            "MongoDB compiler expected a DOL Truth expression",
        ))
    }
}

fn truth_datum(value: Bson) -> BsonDatum {
    BsonDatum {
        state: literal(STATE_VALUE),
        value,
        ty: TypeDef::scalar("dol/truth", 1, ScalarRepr::Truth),
        always_value: true,
    }
}

fn source_state(value: Bson) -> Bson {
    let bson_type = unary_document("$type", value);
    let branches = vec![
        Bson::Document(doc! {
            "case": eq(bson_type.clone(), literal("missing")),
            "then": STATE_MISSING,
        }),
        Bson::Document(doc! {
            "case": eq(bson_type, literal("null")),
            "then": STATE_NULL,
        }),
    ];
    Bson::Document(doc! {
        "$switch": {
            "branches": Bson::Array(branches),
            "default": STATE_VALUE,
        }
    })
}

fn source_field_valid(presence: Presence, ty: &TypeDef, state: Bson, value: Bson) -> Result<Bson> {
    let presence_valid = match (presence, ty.nullability()) {
        (Presence::Required, Nullability::NonNull) => eq(state.clone(), literal(STATE_VALUE)),
        (Presence::Required, Nullability::Nullable) => {
            op("$ne", vec![state.clone(), literal(STATE_MISSING)])
        }
        (Presence::Optional, Nullability::NonNull) => {
            op("$ne", vec![state.clone(), literal(STATE_NULL)])
        }
        (Presence::Optional, Nullability::Nullable) => literal(true),
    };
    let value_valid = source_value_valid(ty, value)?;
    Ok(and(vec![
        presence_valid,
        cond(eq(state, literal(STATE_VALUE)), value_valid, literal(true)),
    ]))
}

fn source_value_valid(ty: &TypeDef, value: Bson) -> Result<Bson> {
    ensure_supported_type(ty)?;
    let bson_type = unary_document("$type", value.clone());
    let valid = match ty.shape() {
        TypeShape::Scalar(ScalarRepr::Bool) => eq(bson_type, literal("bool")),
        TypeShape::Scalar(ScalarRepr::Truth) => {
            let numeric = or(vec![
                eq(bson_type.clone(), literal("int")),
                eq(bson_type, literal("long")),
            ]);
            cond(
                numeric,
                op(
                    "$in",
                    vec![
                        value,
                        Bson::Array(vec![
                            Bson::Int32(TRUTH_FALSE),
                            Bson::Int32(TRUTH_TRUE),
                            Bson::Int32(TRUTH_UNKNOWN),
                        ]),
                    ],
                ),
                literal(false),
            )
        }
        TypeShape::Scalar(ScalarRepr::Int { bits }) if *bits <= 64 => {
            let numeric = or(vec![
                eq(bson_type.clone(), literal("int")),
                eq(bson_type, literal("long")),
            ]);
            let (minimum, maximum) = signed_range(*bits)?;
            cond(
                numeric,
                and(vec![
                    op("$gte", vec![value.clone(), literal(minimum)]),
                    op("$lte", vec![value, literal(maximum)]),
                ]),
                literal(false),
            )
        }
        TypeShape::Scalar(ScalarRepr::UInt { bits }) if *bits <= 64 => {
            let numeric = eq(bson_type, literal("decimal"));
            let maximum = unsigned_max(*bits)?;
            cond(
                numeric,
                and(vec![
                    op("$gte", vec![value.clone(), decimal_literal(0_u128)?]),
                    op("$lte", vec![value, decimal_literal(maximum)?]),
                ]),
                literal(false),
            )
        }
        TypeShape::Scalar(ScalarRepr::Float64) => eq(bson_type, literal("double")),
        TypeShape::Scalar(ScalarRepr::Char) => cond(
            eq(bson_type, literal("string")),
            eq(unary_document("$strLenCP", value), literal(1_i32)),
            literal(false),
        ),
        TypeShape::Scalar(ScalarRepr::String) => eq(bson_type, literal("string")),
        TypeShape::Scalar(ScalarRepr::Bytes) => eq(bson_type, literal("binData")),
        TypeShape::Scalar(ScalarRepr::Uuid) => cond(
            eq(bson_type, literal("string")),
            Bson::Document(doc! {
                "$regexMatch": {
                    "input": value,
                    "regex": "^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$",
                }
            }),
            literal(false),
        ),
        _ => {
            return Err(Diagnostic::error(
                "MONGODB-CODEC-001",
                "unsupported BSON type",
            ));
        }
    };
    Ok(valid)
}

fn signed_range(bits: u8) -> Result<(i64, i64)> {
    if bits == 0 || bits > 64 {
        return Err(Diagnostic::error(
            "MONGODB-CODEC-003",
            "signed integer bit width is outside the BSON contract",
        ));
    }
    if bits == 64 {
        Ok((i64::MIN, i64::MAX))
    } else {
        let magnitude = 1_i64 << (bits - 1);
        Ok((-magnitude, magnitude - 1))
    }
}

fn unsigned_max(bits: u8) -> Result<u128> {
    if bits == 0 || bits > 64 {
        return Err(Diagnostic::error(
            "MONGODB-CODEC-003",
            "unsigned integer bit width is outside the BSON contract",
        ));
    }
    Ok((1_u128 << bits) - 1)
}

fn decimal_literal(value: u128) -> Result<Bson> {
    let value = value
        .to_string()
        .parse::<Decimal128>()
        .map_err(|_| Diagnostic::error("MONGODB-CODEC-003", "integer is outside Decimal128"))?;
    Ok(literal_bson(Bson::Decimal128(value)))
}

fn truth_order_rank(value: Bson) -> Bson {
    cond(
        eq(value.clone(), literal(TRUTH_TRUE)),
        literal(0_i32),
        cond(
            eq(value, literal(TRUTH_FALSE)),
            literal(1_i32),
            literal(2_i32),
        ),
    )
}

fn not_nan(value: Bson) -> Bson {
    op(
        "$ne",
        vec![unary_document("$toString", value), literal("NaN")],
    )
}

fn relation(relations: &[Relation], id: PlanId) -> Result<&Relation> {
    relations.get(id.index()).ok_or_else(|| {
        Diagnostic::error(
            "MONGODB-COMPILE-016",
            "logical plan references a non-topological MongoDB input",
        )
    })
}

fn value(values: &[BsonDatum], index: usize) -> Result<BsonDatum> {
    values.get(index).cloned().ok_or_else(invalid_expression)
}

fn field_path(field: &str) -> Bson {
    Bson::String(format!("${field}"))
}

fn literal<T>(value: T) -> Bson
where
    T: Into<Bson>,
{
    literal_bson(value.into())
}

fn literal_bson(value: Bson) -> Bson {
    Bson::Document(doc! { "$literal": value })
}

fn op(name: &str, arguments: Vec<Bson>) -> Bson {
    let mut document = Document::new();
    document.insert(name, Bson::Array(arguments));
    Bson::Document(document)
}

fn unary_document(name: &str, argument: Bson) -> Bson {
    let mut document = Document::new();
    document.insert(name, argument);
    Bson::Document(document)
}

fn eq(left: Bson, right: Bson) -> Bson {
    op("$eq", vec![left, right])
}

fn and(arguments: Vec<Bson>) -> Bson {
    match arguments.as_slice() {
        [] => literal(true),
        [single] => single.clone(),
        _ => op("$and", arguments),
    }
}

fn or(arguments: Vec<Bson>) -> Bson {
    match arguments.as_slice() {
        [] => literal(false),
        [single] => single.clone(),
        _ => op("$or", arguments),
    }
}

fn cond(predicate: Bson, when_true: Bson, when_false: Bson) -> Bson {
    op("$cond", vec![predicate, when_true, when_false])
}

fn slice_range_error() -> Diagnostic {
    Diagnostic::error(
        "MONGODB-COMPILE-017",
        "MongoDB skip/limit cannot represent this u64 value",
    )
}

fn invalid_expression() -> Diagnostic {
    Diagnostic::error(
        "MONGODB-COMPILE-018",
        "logical expression contains an invalid MongoDB node reference",
    )
}

fn newer_expression() -> Diagnostic {
    Diagnostic::error(
        "MONGODB-COMPILE-019",
        "logical expression operation is newer than this MongoDB compiler contract",
    )
}
