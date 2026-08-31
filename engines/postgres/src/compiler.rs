//! Exact offline SQL compilation for the first PostgreSQL slice.

use dol_core::diagnostic::{Diagnostic, Result};
use dol_core::expr::Parameters;
use dol_core::model::{ModelDef, ModelKey, Presence};
use dol_core::plan::{
    LogicalBinaryOp, LogicalExpr, LogicalExprNodeKind, LogicalNode, LogicalPlan, LogicalUnaryOp,
    PlanId, PlanOutput, ProjectionKind,
};
use dol_core::types::Nullability;
use dol_core::types::{ScalarRepr, TypeDef, TypeShape};
use dol_core::value::{Datum, validate_datum};

use crate::mapping::{PostgresCatalog, quote_identifier};
use crate::sql::{CompiledQuery, SqlBind, SqlOutputColumn};

mod expression;

use expression::{
    SqlDatum, compile_binary, compile_conditional, compile_membership, compile_unary,
    ensure_truth_type,
};

const STATE_MISSING: i16 = 0;
const STATE_NULL: i16 = 1;
const STATE_VALUE: i16 = 2;
const TRUTH_FALSE: i16 = 0;
const TRUTH_TRUE: i16 = 1;
const TRUTH_UNKNOWN: i16 = 2;

/// Offline PostgreSQL compiler over validated DOL logical plans.
#[derive(Debug, Clone, Copy)]
pub struct PostgresCompiler<'a> {
    catalog: &'a PostgresCatalog,
}

impl<'a> PostgresCompiler<'a> {
    /// Creates a compiler borrowing immutable physical mappings.
    #[must_use]
    pub const fn new(catalog: &'a PostgresCatalog) -> Self {
        Self { catalog }
    }

    pub(crate) fn supports_node(&self, node: &LogicalNode) -> bool {
        match node {
            LogicalNode::Source { model, .. } => {
                self.catalog.table(model).is_ok()
                    && model
                        .fields()
                        .iter()
                        .all(|field| postgres_type(field.ty()).is_ok())
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

    /// Compiles one exact Slice-1 plan to parameterized PostgreSQL SQL.
    pub fn compile(&self, plan: &LogicalPlan, parameters: &Parameters) -> Result<CompiledQuery> {
        let mut context = CompileContext {
            catalog: self.catalog,
            parameters,
            binds: Vec::new(),
            next_relation: 0,
        };
        let mut relations = Vec::with_capacity(plan.nodes().len());
        for node in plan.nodes() {
            let relation = match node {
                LogicalNode::Source { model, .. } => context.compile_source(model)?,
                LogicalNode::Filter { input, condition } => {
                    let relation = compiled_relation(&relations, *input)?.clone();
                    context.compile_filter(relation, condition)?
                }
                LogicalNode::Project { input, projection } => {
                    let relation = compiled_relation(&relations, *input)?.clone();
                    context.compile_project(relation, projection)?
                }
                LogicalNode::Slice {
                    input,
                    offset,
                    limit,
                } => {
                    let relation = compiled_relation(&relations, *input)?.clone();
                    context.compile_slice(relation, *offset, *limit)?
                }
                _ => {
                    return Err(Diagnostic::error(
                        "POSTGRES-COMPILE-003",
                        "Stage H compiler supports only source/filter/project/slice",
                    ));
                }
            };
            relations.push(relation);
        }
        let relation = compiled_relation(&relations, plan.root())?.clone();
        if &relation.output != plan.output() {
            return Err(Diagnostic::error(
                "POSTGRES-COMPILE-001",
                "compiled PostgreSQL output shape differs from the logical plan",
            ));
        }
        let columns = relation
            .output_columns
            .iter()
            .map(|column| {
                SqlOutputColumn::new(
                    column.state_alias.clone(),
                    column.value_alias.clone(),
                    column.ty.clone(),
                )
            })
            .collect();
        let sql = transport_output_sql(&relation)?;
        Ok(CompiledQuery::new(
            sql,
            context.binds,
            columns,
            relation.output,
        ))
    }
}

fn expression_support(expression: &LogicalExpr) -> Option<bool> {
    let view = expression.engine_view();
    if view.outer_scope_count() != 0 {
        return None;
    }
    let mut always_values = Vec::with_capacity(view.nodes().len());
    for node in view.nodes() {
        if postgres_type(node.type_def()).is_err()
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

struct CompileContext<'a> {
    catalog: &'a PostgresCatalog,
    parameters: &'a Parameters,
    binds: Vec<SqlBind>,
    next_relation: usize,
}

impl CompileContext<'_> {
    fn compile_source(&mut self, model: &ModelDef) -> Result<Relation> {
        let mapping = self.catalog.table(model)?;
        let table_alias = self.next_alias("t");
        let qualified = format!(
            "{}.{}",
            quote_identifier(mapping.schema())?,
            quote_identifier(mapping.table())?
        );
        let qtable = quote_identifier(&table_alias)?;
        let mut selects = Vec::with_capacity(model.fields().len() * 2);
        let mut fields = Vec::with_capacity(model.fields().len());
        let mut output_columns = Vec::with_capacity(model.fields().len());

        for (index, field) in model.fields().iter().enumerate() {
            postgres_type(field.ty())?;
            let column = mapping.field_mapping(field.key().as_str()).ok_or_else(|| {
                Diagnostic::error(
                    "POSTGRES-MAP-002",
                    format!(
                        "field `{}` has no PostgreSQL column mapping",
                        field.key().as_str()
                    ),
                )
            })?;
            let value_column = format!("{qtable}.{}", quote_identifier(column.value())?);
            let state_alias = format!("__dol_s0_f{index}_state");
            let value_alias = format!("__dol_s0_f{index}_value");
            let state_sql = source_state_sql(
                field.presence(),
                field.ty(),
                column.presence(),
                &qtable,
                &value_column,
            )?;
            selects.push(format!(
                "{state_sql} AS {}",
                quote_identifier(&state_alias)?
            ));
            selects.push(format!(
                "{value_column} AS {}",
                quote_identifier(&value_alias)?
            ));
            let always_value = field.presence() == Presence::Required
                && field.ty().nullability() == Nullability::NonNull;
            fields.push(ColumnRef {
                state_alias: state_alias.clone(),
                value_alias: value_alias.clone(),
                ty: field.ty().clone(),
                always_value,
            });
            output_columns.push(ColumnRef {
                state_alias,
                value_alias,
                ty: field.ty().clone(),
                always_value,
            });
        }

        Ok(Relation {
            sql: format!("SELECT {} FROM {qualified} AS {qtable}", selects.join(", ")),
            scopes: vec![ScopeLayout {
                model_key: model.key().clone(),
                fields,
            }],
            output_columns,
            output: PlanOutput::Model(Box::new(model.clone())),
        })
    }

    fn compile_filter(&mut self, relation: Relation, condition: &LogicalExpr) -> Result<Relation> {
        let alias = self.next_alias("q");
        let qal = quote_identifier(&alias)?;
        let truth = self.compile_expr(condition, &relation, &qal)?;
        ensure_truth_type(condition.type_def())?;
        if !truth.always_value {
            return Err(Diagnostic::error(
                "POSTGRES-COMPILE-004",
                "filter truth expression can be Missing/Null; the current PostgreSQL compiler cannot reproduce that runtime type error exactly",
            ));
        }
        let sql = format!(
            "SELECT * FROM ({}) AS {qal} WHERE ({}) = {STATE_VALUE} AND ({}) = {TRUTH_TRUE}",
            relation.sql, truth.state, truth.value
        );
        Ok(Relation { sql, ..relation })
    }

    fn compile_project(
        &mut self,
        relation: Relation,
        projection: &dol_core::plan::LogicalProjection,
    ) -> Result<Relation> {
        let alias = self.next_alias("q");
        let qal = quote_identifier(&alias)?;
        let mut selects = Vec::new();
        let mut output_columns = Vec::new();
        for (index, expression) in projection.expressions().iter().enumerate() {
            let datum = self.compile_expr(expression, &relation, &qal)?;
            let state_alias = format!("__dol_v{index}_state");
            let value_alias = format!("__dol_v{index}_value");
            selects.push(format!(
                "{} AS {}",
                datum.state,
                quote_identifier(&state_alias)?
            ));
            selects.push(format!(
                "{} AS {}",
                datum.value,
                quote_identifier(&value_alias)?
            ));
            output_columns.push(ColumnRef {
                state_alias,
                value_alias,
                ty: expression.type_def().clone(),
                always_value: datum.always_value,
            });
        }
        let expected = match projection.kind() {
            ProjectionKind::Value => 1,
            ProjectionKind::Tuple | ProjectionKind::Record => projection.expressions().len(),
        };
        if output_columns.len() != expected {
            return Err(Diagnostic::error(
                "POSTGRES-COMPILE-005",
                "projection output arity changed during PostgreSQL compilation",
            ));
        }
        Ok(Relation {
            sql: format!(
                "SELECT {} FROM ({}) AS {qal}",
                selects.join(", "),
                relation.sql
            ),
            scopes: Vec::new(),
            output_columns,
            output: PlanOutput::Value(projection.type_def().clone()),
        })
    }

    fn compile_slice(
        &mut self,
        relation: Relation,
        offset: u64,
        limit: Option<u64>,
    ) -> Result<Relation> {
        let offset = i64::try_from(offset).map_err(|_| {
            Diagnostic::error(
                "POSTGRES-COMPILE-018",
                "PostgreSQL OFFSET cannot represent this DOL u64 offset exactly",
            )
        })?;
        let limit = limit
            .map(|value| {
                i64::try_from(value).map_err(|_| {
                    Diagnostic::error(
                        "POSTGRES-COMPILE-018",
                        "PostgreSQL LIMIT cannot represent this DOL u64 limit exactly",
                    )
                })
            })
            .transpose()?;
        let alias = self.next_alias("q");
        let qal = quote_identifier(&alias)?;
        let mut sql = format!("SELECT * FROM ({}) AS {qal}", relation.sql);
        if let Some(limit) = limit {
            sql.push_str(&format!(" LIMIT {limit}"));
        }
        if offset != 0 {
            sql.push_str(&format!(" OFFSET {offset}"));
        }
        Ok(Relation { sql, ..relation })
    }

    fn compile_expr(
        &mut self,
        expression: &LogicalExpr,
        relation: &Relation,
        qualifier: &str,
    ) -> Result<SqlDatum> {
        let view = expression.engine_view();
        if view.outer_scope_count() != 0 {
            return Err(Diagnostic::error(
                "POSTGRES-COMPILE-006",
                "correlated expressions are not compiled by PostgreSQL Stage H Slice 1",
            ));
        }
        let mut values: Vec<SqlDatum> = Vec::with_capacity(view.nodes().len());
        for node in view.nodes() {
            let datum = match node.kind() {
                LogicalExprNodeKind::Field {
                    scope,
                    slot,
                    allow_null_extension,
                } => {
                    if *allow_null_extension {
                        return Err(Diagnostic::error(
                            "POSTGRES-COMPILE-007",
                            "outer-join null-extension is not part of PostgreSQL Stage H Slice 1",
                        ));
                    }
                    let expected = view.scopes().get(*scope).ok_or_else(invalid_expr)?;
                    let scope_layout = relation.scopes.get(*scope).ok_or_else(|| {
                        Diagnostic::error(
                            "POSTGRES-COMPILE-008",
                            "expression refers to a model scope no longer available after projection",
                        )
                    })?;
                    if expected != &scope_layout.model_key {
                        return Err(Diagnostic::error(
                            "POSTGRES-COMPILE-009",
                            "prepared expression scope does not match PostgreSQL relation scope",
                        ));
                    }
                    let field = scope_layout.fields.get(*slot).ok_or_else(invalid_expr)?;
                    SqlDatum {
                        state: qualify(qualifier, &field.state_alias)?,
                        value: qualify(qualifier, &field.value_alias)?,
                        ty: node.type_def().clone(),
                        always_value: field.always_value,
                    }
                }
                LogicalExprNodeKind::Literal(datum) => {
                    self.compile_bound_datum(node.type_def(), (**datum).clone())?
                }
                LogicalExprNodeKind::Parameter(name) => {
                    let parameter = self.parameters.get(name).ok_or_else(|| {
                        Diagnostic::error(
                            "POSTGRES-PARAM-001",
                            format!("parameter `{name}` is not bound"),
                        )
                    })?;
                    if parameter.type_def() != node.type_def() {
                        return Err(Diagnostic::error(
                            "POSTGRES-PARAM-002",
                            format!("parameter `{name}` was bound with a different semantic type"),
                        ));
                    }
                    self.compile_bound_datum(node.type_def(), parameter.datum().clone())?
                }
                LogicalExprNodeKind::Unary { op, input } => {
                    let input = get_expr(&values, *input)?;
                    compile_unary(*op, input, node.type_def())?
                }
                LogicalExprNodeKind::Binary { op, left, right } => {
                    let left = get_expr(&values, *left)?;
                    let right = get_expr(&values, *right)?;
                    compile_binary(*op, left, right, node.type_def())?
                }
                LogicalExprNodeKind::Membership {
                    input,
                    candidates,
                    negate,
                } => {
                    let input = get_expr(&values, *input)?;
                    let candidates = candidates
                        .iter()
                        .map(|index| get_expr(&values, *index))
                        .collect::<Result<Vec<_>>>()?;
                    compile_membership(input, &candidates, *negate)?
                }
                LogicalExprNodeKind::Conditional {
                    condition,
                    when_true,
                    when_false,
                } => compile_conditional(
                    get_expr(&values, *condition)?,
                    get_expr(&values, *when_true)?,
                    get_expr(&values, *when_false)?,
                    node.type_def(),
                )?,
                LogicalExprNodeKind::FunctionCall { .. } => {
                    return Err(Diagnostic::error(
                        "POSTGRES-COMPILE-010",
                        "semantic function SQL mappings arrive after the Stage H Slice-1 compiler contract",
                    ));
                }
                LogicalExprNodeKind::Exists { .. } => {
                    return Err(Diagnostic::error(
                        "POSTGRES-COMPILE-011",
                        "existential/correlated SQL compilation is not part of PostgreSQL Stage H Slice 1",
                    ));
                }
                _ => {
                    return Err(Diagnostic::error(
                        "POSTGRES-COMPILE-019",
                        "logical expression node is newer than this PostgreSQL compiler's exact lowering contract",
                    ));
                }
            };
            values.push(datum);
        }
        values.get(view.root()).cloned().ok_or_else(invalid_expr)
    }

    fn compile_bound_datum(&mut self, ty: &TypeDef, datum: Datum) -> Result<SqlDatum> {
        validate_datum(ty, Presence::Optional, &datum).map_err(|error| {
            Diagnostic::error(
                "POSTGRES-PARAM-003",
                format!(
                    "PostgreSQL bind contains an invalid semantic datum: {}",
                    error.message()
                ),
            )
        })?;
        let state = match &datum {
            Datum::Missing => STATE_MISSING,
            Datum::Null => STATE_NULL,
            Datum::Value(_) => STATE_VALUE,
        };
        self.binds.push(SqlBind::state(state));
        let state_placeholder = format!("${}::smallint", self.binds.len());
        self.binds.push(SqlBind::datum(ty.clone(), datum.clone()));
        let value_placeholder = postgres_bind_placeholder(ty, self.binds.len())?;
        Ok(SqlDatum {
            state: state_placeholder,
            value: value_placeholder,
            ty: ty.clone(),
            always_value: matches!(datum, Datum::Value(_)),
        })
    }

    fn next_alias(&mut self, prefix: &str) -> String {
        let value = format!("__dol_{prefix}{}", self.next_relation);
        self.next_relation += 1;
        value
    }
}

#[derive(Clone)]
struct ColumnRef {
    state_alias: String,
    value_alias: String,
    ty: TypeDef,
    always_value: bool,
}

#[derive(Clone)]
struct ScopeLayout {
    model_key: ModelKey,
    fields: Vec<ColumnRef>,
}

#[derive(Clone)]
struct Relation {
    sql: String,
    scopes: Vec<ScopeLayout>,
    output_columns: Vec<ColumnRef>,
    output: PlanOutput,
}

fn compiled_relation(relations: &[Relation], id: PlanId) -> Result<&Relation> {
    relations.get(id.index()).ok_or_else(|| {
        Diagnostic::error(
            "POSTGRES-COMPILE-002",
            "logical plan references a non-topological PostgreSQL compilation node",
        )
    })
}

fn source_state_sql(
    presence: Presence,
    ty: &TypeDef,
    presence_column: Option<&str>,
    qualifier: &str,
    value_sql: &str,
) -> Result<String> {
    match presence {
        Presence::Required => Ok(match ty.nullability() {
            Nullability::NonNull => STATE_VALUE.to_string(),
            Nullability::Nullable => {
                format!("CASE WHEN {value_sql} IS NULL THEN {STATE_NULL} ELSE {STATE_VALUE} END")
            }
        }),
        Presence::Optional => {
            let presence = presence_column.ok_or_else(|| {
                Diagnostic::error(
                    "POSTGRES-MAP-004",
                    "optional field requires a presence column",
                )
            })?;
            let present_sql = format!("{qualifier}.{}", quote_identifier(presence)?);
            Ok(match ty.nullability() {
                Nullability::NonNull => format!(
                    "CASE WHEN {present_sql} = FALSE THEN {STATE_MISSING} ELSE {STATE_VALUE} END"
                ),
                Nullability::Nullable => format!(
                    "CASE WHEN {present_sql} = FALSE THEN {STATE_MISSING} WHEN {value_sql} IS NULL THEN {STATE_NULL} ELSE {STATE_VALUE} END"
                ),
            })
        }
    }
}

pub(crate) fn scalar_repr(ty: &TypeDef) -> Result<ScalarRepr> {
    match ty.shape() {
        TypeShape::Scalar(repr) => Ok(*repr),
        _ => Err(Diagnostic::error(
            "POSTGRES-TYPE-001",
            "PostgreSQL compiler currently accepts only scalar expression values",
        )),
    }
}

pub(crate) fn postgres_type(ty: &TypeDef) -> Result<&'static str> {
    match scalar_repr(ty)? {
        ScalarRepr::Bool => Ok("boolean"),
        ScalarRepr::Truth => Ok("smallint"),
        ScalarRepr::Int { bits } if bits <= 16 => Ok("smallint"),
        ScalarRepr::Int { bits } if bits <= 32 => Ok("integer"),
        ScalarRepr::Int { bits } if bits <= 64 => Ok("bigint"),
        ScalarRepr::Int { .. } | ScalarRepr::UInt { .. } | ScalarRepr::Decimal => Ok("numeric"),
        ScalarRepr::Float32 => Ok("real"),
        ScalarRepr::Float64 => Ok("double precision"),
        ScalarRepr::Char | ScalarRepr::String => Ok("text"),
        ScalarRepr::Bytes => Ok("bytea"),
        ScalarRepr::Uuid => Ok("uuid"),
        ScalarRepr::Date => Ok("date"),
        ScalarRepr::Time | ScalarRepr::LocalDateTime | ScalarRepr::Instant => {
            Err(Diagnostic::error(
                "POSTGRES-TYPE-002",
                "DOL subsecond temporal values are not mapped to native PostgreSQL timestamp/time types until precision loss is explicitly eliminated",
            ))
        }
        ScalarRepr::Duration => Err(Diagnostic::error(
            "POSTGRES-TYPE-004",
            "DOL Duration is not mapped to PostgreSQL interval until month/day ambiguity is explicitly excluded",
        )),
        _ => Err(Diagnostic::error(
            "POSTGRES-TYPE-003",
            "PostgreSQL scalar representation is not supported by the current compiler",
        )),
    }
}

fn postgres_bind_placeholder(ty: &TypeDef, index: usize) -> Result<String> {
    match scalar_repr(ty)? {
        ScalarRepr::Int { bits } if bits > 64 => Ok(format!("(${index}::text)::numeric")),
        ScalarRepr::UInt { .. } | ScalarRepr::Decimal => Ok(format!("(${index}::text)::numeric")),
        _ => Ok(format!("${index}::{}", postgres_type(ty)?)),
    }
}

fn transport_output_sql(relation: &Relation) -> Result<String> {
    let alias = quote_identifier("__dol_out")?;
    let mut selects = Vec::with_capacity(relation.output_columns.len() * 2);
    for column in &relation.output_columns {
        let state = qualify(&alias, &column.state_alias)?;
        let value = qualify(&alias, &column.value_alias)?;
        selects.push(format!(
            "({state})::smallint AS {}",
            quote_identifier(&column.state_alias)?
        ));
        selects.push(format!(
            "{} AS {}",
            transport_value_sql(&column.ty, &value)?,
            quote_identifier(&column.value_alias)?
        ));
    }
    Ok(format!(
        "SELECT {} FROM ({}) AS {alias}",
        selects.join(", "),
        relation.sql
    ))
}

fn transport_value_sql(ty: &TypeDef, value: &str) -> Result<String> {
    let sql = match scalar_repr(ty)? {
        ScalarRepr::Bool => format!("({value})::boolean"),
        ScalarRepr::Truth => format!("({value})::smallint"),
        ScalarRepr::Int { bits } if bits <= 16 => format!("({value})::smallint"),
        ScalarRepr::Int { bits } if bits <= 32 => format!("({value})::integer"),
        ScalarRepr::Int { bits } if bits <= 64 => format!("({value})::bigint"),
        ScalarRepr::Int { .. } | ScalarRepr::UInt { .. } | ScalarRepr::Decimal => {
            format!("({value})::text")
        }
        ScalarRepr::Float32 => format!("({value})::real"),
        ScalarRepr::Float64 => format!("({value})::double precision"),
        ScalarRepr::Char | ScalarRepr::String => format!("({value})::text"),
        ScalarRepr::Bytes => format!("({value})::bytea"),
        ScalarRepr::Uuid => format!("({value})::uuid"),
        ScalarRepr::Date => format!("({value})::date"),
        _ => {
            return Err(Diagnostic::error(
                "POSTGRES-TYPE-003",
                "PostgreSQL runtime transport cannot decode this scalar representation",
            ));
        }
    };
    Ok(sql)
}

fn qualify(qualifier: &str, alias: &str) -> Result<String> {
    Ok(format!("{qualifier}.{}", quote_identifier(alias)?))
}

fn get_expr(values: &[SqlDatum], index: usize) -> Result<SqlDatum> {
    values.get(index).cloned().ok_or_else(invalid_expr)
}

fn invalid_expr() -> Diagnostic {
    Diagnostic::error(
        "POSTGRES-COMPILE-017",
        "prepared expression contains a non-topological or invalid node reference",
    )
}
