//! Scalar expression lowering for the offline PostgreSQL compiler.

use dol_core::diagnostic::{Diagnostic, Result};
use dol_core::plan::{LogicalBinaryOp, LogicalUnaryOp};
use dol_core::types::{ScalarRepr, TypeDef, TypeShape};

use super::{
    STATE_MISSING, STATE_NULL, STATE_VALUE, TRUTH_FALSE, TRUTH_TRUE, TRUTH_UNKNOWN, scalar_repr,
};

#[derive(Clone)]
pub(super) struct SqlDatum {
    pub(super) state: String,
    pub(super) value: String,
    pub(super) ty: TypeDef,
    pub(super) always_value: bool,
}

pub(super) fn compile_unary(
    op: LogicalUnaryOp,
    input: SqlDatum,
    output: &TypeDef,
) -> Result<SqlDatum> {
    match op {
        LogicalUnaryOp::IsNull => Ok(truth_datum(format!(
            "CASE WHEN ({}) = {STATE_NULL} THEN {TRUTH_TRUE} ELSE {TRUTH_FALSE} END",
            input.state
        ))),
        LogicalUnaryOp::IsMissing => Ok(truth_datum(format!(
            "CASE WHEN ({}) = {STATE_MISSING} THEN {TRUTH_TRUE} ELSE {TRUTH_FALSE} END",
            input.state
        ))),
        LogicalUnaryOp::IsPresent => Ok(truth_datum(format!(
            "CASE WHEN ({}) = {STATE_MISSING} THEN {TRUTH_FALSE} ELSE {TRUTH_TRUE} END",
            input.state
        ))),
        LogicalUnaryOp::NullableLift => Ok(SqlDatum {
            ty: output.clone(),
            ..input
        }),
        LogicalUnaryOp::Not => {
            require_total_truth(&input)?;
            Ok(truth_datum(format!(
                "CASE ({}) WHEN {TRUTH_TRUE} THEN {TRUTH_FALSE} WHEN {TRUTH_FALSE} THEN {TRUTH_TRUE} WHEN {TRUTH_UNKNOWN} THEN {TRUTH_UNKNOWN} ELSE NULL::smallint END",
                input.value
            )))
        }
        LogicalUnaryOp::LosslessCast | LogicalUnaryOp::Negate | LogicalUnaryOp::Abs => {
            Err(Diagnostic::error(
                "POSTGRES-COMPILE-012",
                "numeric casts/unary arithmetic are deferred beyond PostgreSQL Stage H Slice 1",
            ))
        }
        _ => Err(Diagnostic::error(
            "POSTGRES-COMPILE-019",
            "unary expression operation is newer than this PostgreSQL compiler's exact lowering contract",
        )),
    }
}

pub(super) fn compile_binary(
    op: LogicalBinaryOp,
    left: SqlDatum,
    right: SqlDatum,
    output: &TypeDef,
) -> Result<SqlDatum> {
    match op {
        LogicalBinaryOp::Eq | LogicalBinaryOp::Ne => {
            let equal = value_equal_sql(&left, &right)?;
            let value = if op == LogicalBinaryOp::Eq {
                equal
            } else {
                format!("NOT ({equal})")
            };
            Ok(comparison_truth(&left, &right, value))
        }
        LogicalBinaryOp::Lt | LogicalBinaryOp::Le | LogicalBinaryOp::Gt | LogicalBinaryOp::Ge => {
            let value = value_order_sql(op, &left, &right)?;
            Ok(comparison_truth(&left, &right, value))
        }
        LogicalBinaryOp::IsDistinctFrom | LogicalBinaryOp::IsNotDistinctFrom => {
            let value_equal = value_equal_sql(&left, &right)?;
            let equal = format!(
                "CASE WHEN ({ls}) = {STATE_MISSING} AND ({rs}) = {STATE_MISSING} THEN TRUE \
                 WHEN ({ls}) = {STATE_NULL} AND ({rs}) = {STATE_NULL} THEN TRUE \
                 WHEN ({ls}) <> {STATE_VALUE} OR ({rs}) <> {STATE_VALUE} THEN FALSE \
                 ELSE ({value_equal}) END",
                ls = left.state,
                rs = right.state,
            );
            let result = if op == LogicalBinaryOp::IsNotDistinctFrom {
                equal
            } else {
                format!("NOT ({equal})")
            };
            Ok(truth_datum(format!(
                "CASE WHEN {} THEN {TRUTH_TRUE} ELSE {TRUTH_FALSE} END",
                result
            )))
        }
        LogicalBinaryOp::And | LogicalBinaryOp::Or => {
            require_total_truth(&left)?;
            require_total_truth(&right)?;
            let value = if op == LogicalBinaryOp::And {
                format!(
                    "CASE WHEN ({l}) = {TRUTH_FALSE} OR ({r}) = {TRUTH_FALSE} THEN {TRUTH_FALSE} \
                     WHEN ({l}) = {TRUTH_TRUE} AND ({r}) = {TRUTH_TRUE} THEN {TRUTH_TRUE} ELSE {TRUTH_UNKNOWN} END",
                    l = left.value,
                    r = right.value,
                )
            } else {
                format!(
                    "CASE WHEN ({l}) = {TRUTH_TRUE} OR ({r}) = {TRUTH_TRUE} THEN {TRUTH_TRUE} \
                     WHEN ({l}) = {TRUTH_FALSE} AND ({r}) = {TRUTH_FALSE} THEN {TRUTH_FALSE} ELSE {TRUTH_UNKNOWN} END",
                    l = left.value,
                    r = right.value,
                )
            };
            Ok(truth_datum(value))
        }
        LogicalBinaryOp::Coalesce => Ok(SqlDatum {
            state: format!(
                "CASE WHEN ({}) = {STATE_VALUE} THEN ({}) ELSE ({}) END",
                left.state, left.state, right.state
            ),
            value: format!(
                "CASE WHEN ({}) = {STATE_VALUE} THEN ({}) ELSE ({}) END",
                left.state, left.value, right.value
            ),
            ty: output.clone(),
            always_value: left.always_value || right.always_value,
        }),
        LogicalBinaryOp::Add
        | LogicalBinaryOp::Sub
        | LogicalBinaryOp::Mul
        | LogicalBinaryOp::Div
        | LogicalBinaryOp::Rem => Err(Diagnostic::error(
            "POSTGRES-COMPILE-013",
            "checked arithmetic SQL lowering is deferred beyond PostgreSQL Stage H Slice 1",
        )),
        _ => Err(Diagnostic::error(
            "POSTGRES-COMPILE-019",
            "binary expression operation is newer than this PostgreSQL compiler's exact lowering contract",
        )),
    }
}

pub(super) fn compile_membership(
    input: SqlDatum,
    candidates: &[SqlDatum],
    negate: bool,
) -> Result<SqlDatum> {
    let mut comparisons = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        comparisons.push(comparison_truth(
            &input,
            candidate,
            value_equal_sql(&input, candidate)?,
        ));
    }
    let saw_true = comparisons
        .iter()
        .map(|value| format!("({}) = {TRUTH_TRUE}", value.value))
        .collect::<Vec<_>>()
        .join(" OR ");
    let saw_unknown = comparisons
        .iter()
        .map(|value| format!("({}) = {TRUTH_UNKNOWN}", value.value))
        .collect::<Vec<_>>()
        .join(" OR ");
    let true_value = if negate { TRUTH_FALSE } else { TRUTH_TRUE };
    let false_value = if negate { TRUTH_TRUE } else { TRUTH_FALSE };
    let value = if comparisons.is_empty() {
        false_value.to_string()
    } else {
        format!(
            "CASE WHEN {saw_true} THEN {true_value} WHEN {saw_unknown} THEN {TRUTH_UNKNOWN} ELSE {false_value} END"
        )
    };
    Ok(truth_datum(value))
}

pub(super) fn compile_conditional(
    condition: SqlDatum,
    when_true: SqlDatum,
    when_false: SqlDatum,
    output: &TypeDef,
) -> Result<SqlDatum> {
    require_total_truth(&condition)?;
    Ok(SqlDatum {
        state: format!(
            "CASE WHEN ({}) = {TRUTH_TRUE} THEN ({}) ELSE ({}) END",
            condition.value, when_true.state, when_false.state
        ),
        value: format!(
            "CASE WHEN ({}) = {TRUTH_TRUE} THEN ({}) ELSE ({}) END",
            condition.value, when_true.value, when_false.value
        ),
        ty: output.clone(),
        always_value: when_true.always_value && when_false.always_value,
    })
}

fn comparison_truth(left: &SqlDatum, right: &SqlDatum, value_comparison: String) -> SqlDatum {
    truth_datum(format!(
        "CASE WHEN ({ls}) <> {STATE_VALUE} OR ({rs}) <> {STATE_VALUE} THEN {TRUTH_UNKNOWN} \
         WHEN ({comparison}) THEN {TRUTH_TRUE} ELSE {TRUTH_FALSE} END",
        ls = left.state,
        rs = right.state,
        comparison = value_comparison,
    ))
}

fn value_equal_sql(left: &SqlDatum, right: &SqlDatum) -> Result<String> {
    ensure_same_scalar_shape(&left.ty, &right.ty)?;
    match scalar_repr(&left.ty)? {
        ScalarRepr::Float32 => Ok(format!(
            "CASE WHEN ({l}) = 'NaN'::real OR ({r}) = 'NaN'::real THEN FALSE ELSE ({l}) = ({r}) END",
            l = left.value,
            r = right.value,
        )),
        ScalarRepr::Float64 => Ok(format!(
            "CASE WHEN ({l}) = 'NaN'::double precision OR ({r}) = 'NaN'::double precision THEN FALSE ELSE ({l}) = ({r}) END",
            l = left.value,
            r = right.value,
        )),
        ScalarRepr::String | ScalarRepr::Char => Ok(format!(
            "(({}) COLLATE \"C\") = (({}) COLLATE \"C\")",
            left.value, right.value
        )),
        _ => Ok(format!("({}) = ({})", left.value, right.value)),
    }
}

fn value_order_sql(op: LogicalBinaryOp, left: &SqlDatum, right: &SqlDatum) -> Result<String> {
    ensure_same_scalar_shape(&left.ty, &right.ty)?;
    let operator = match op {
        LogicalBinaryOp::Lt => "<",
        LogicalBinaryOp::Le => "<=",
        LogicalBinaryOp::Gt => ">",
        LogicalBinaryOp::Ge => ">=",
        _ => {
            return Err(Diagnostic::error(
                "POSTGRES-COMPILE-019",
                "ordering expression operation is newer than this PostgreSQL compiler's exact lowering contract",
            ));
        }
    };
    match scalar_repr(&left.ty)? {
        ScalarRepr::Float32 => Ok(format!(
            "CASE WHEN ({l}) = 'NaN'::real OR ({r}) = 'NaN'::real THEN FALSE ELSE ({l}) {operator} ({r}) END",
            l = left.value,
            r = right.value,
        )),
        ScalarRepr::Float64 => Ok(format!(
            "CASE WHEN ({l}) = 'NaN'::double precision OR ({r}) = 'NaN'::double precision THEN FALSE ELSE ({l}) {operator} ({r}) END",
            l = left.value,
            r = right.value,
        )),
        ScalarRepr::String | ScalarRepr::Char => Ok(format!(
            "(({}) COLLATE \"C\") {operator} (({}) COLLATE \"C\")",
            left.value, right.value
        )),
        _ => Ok(format!("({}) {operator} ({})", left.value, right.value)),
    }
}

fn truth_datum(value: String) -> SqlDatum {
    SqlDatum {
        state: STATE_VALUE.to_string(),
        value,
        ty: TypeDef::scalar("dol/truth", 1, ScalarRepr::Truth),
        always_value: true,
    }
}

fn require_total_truth(value: &SqlDatum) -> Result<()> {
    ensure_truth_type(&value.ty)?;
    if !value.always_value {
        return Err(Diagnostic::error(
            "POSTGRES-COMPILE-014",
            "truth operation could receive Missing/Null and must remain local until exact PostgreSQL error lowering is implemented",
        ));
    }
    Ok(())
}

pub(super) fn ensure_truth_type(ty: &TypeDef) -> Result<()> {
    if matches!(ty.shape(), TypeShape::Scalar(ScalarRepr::Truth)) {
        Ok(())
    } else {
        Err(Diagnostic::error(
            "POSTGRES-COMPILE-015",
            "PostgreSQL compiler expected a DOL Truth expression",
        ))
    }
}

fn ensure_same_scalar_shape(left: &TypeDef, right: &TypeDef) -> Result<()> {
    let left = scalar_repr(left)?;
    let right = scalar_repr(right)?;
    if left == right {
        Ok(())
    } else {
        Err(Diagnostic::error(
            "POSTGRES-COMPILE-016",
            "comparison operands do not share one PostgreSQL scalar representation",
        ))
    }
}
