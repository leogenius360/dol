use std::sync::Arc;

use crate::diagnostic::{Diagnostic, Result};
use crate::limits::{DefinitionLimits, ExpressionLimits};
use crate::semantics::Truth;
use crate::types::{
    DataType, Presence, ScalarRepr, TypeDef, TypeShape, validate_type, validate_type_universe,
};
use crate::value::validate_datum;

use super::function::{FunctionDef, validate_function_def};
use super::node::{BinaryOp, ExprKind, ExprNode, UnaryOp};
use super::semantic::validate_lossless_cast;

pub(crate) fn validate_expression_node(node: &ExprNode, limits: ExpressionLimits) -> Result<()> {
    let mut stack = vec![node];
    let mut types = Vec::new();
    let mut functions = std::collections::BTreeMap::<(String, u32), FunctionDef>::new();
    while let Some(current) = stack.pop() {
        validate_type(&current.ty, DefinitionLimits::default())?;
        types.push(&current.ty);
        match &current.kind {
            ExprKind::Field(_) | ExprKind::Literal(_) | ExprKind::Parameter(_) => {}
            ExprKind::Unary { input, .. } => stack.push(input),
            ExprKind::Binary { left, right, .. } => {
                stack.push(right);
                stack.push(left);
            }
            ExprKind::Membership {
                input, candidates, ..
            } => {
                for candidate in candidates.iter().rev() {
                    stack.push(candidate);
                }
                stack.push(input);
            }
            ExprKind::Conditional {
                condition,
                when_true,
                when_false,
            } => {
                stack.push(when_false);
                stack.push(when_true);
                stack.push(condition);
            }
            ExprKind::Exists(_) => {}
            ExprKind::FunctionCall {
                function,
                arguments,
            } => {
                let definition = function.definition();
                validate_function_def(definition)?;
                let identity = (definition.key().as_str().to_owned(), definition.version());
                if let Some(existing) = functions.get(&identity) {
                    if existing != definition {
                        return Err(Diagnostic::error(
                            "EXPR-FUNC-004",
                            format!(
                                "function `{}` version {} has conflicting semantic definitions",
                                definition.key().as_str(),
                                definition.version()
                            ),
                        ));
                    }
                } else {
                    functions.insert(identity, definition.clone());
                }
                for argument in arguments.iter().rev() {
                    stack.push(argument);
                }
            }
        }
    }
    validate_type_universe(types)?;

    let mut parameters = std::collections::BTreeMap::<Arc<str>, TypeDef>::new();
    validate_node(node, limits, &mut parameters)
}

fn validate_node(
    node: &ExprNode,
    limits: ExpressionLimits,
    parameters: &mut std::collections::BTreeMap<Arc<str>, TypeDef>,
) -> Result<()> {
    match &node.kind {
        ExprKind::Field(_) => {}
        ExprKind::Literal(value) => {
            validate_datum(&node.ty, Presence::Required, value).map_err(|error| {
                Diagnostic::error(
                    "EXPR-LITERAL-001",
                    format!("invalid typed expression literal: {}", error.message()),
                )
            })?;
        }
        ExprKind::Parameter(parameter) => {
            if parameter.name.is_empty() {
                return Err(Diagnostic::error(
                    "EXPR-PARAM-001",
                    "parameter name must not be empty",
                ));
            }
            if parameter.name.len() > limits.max_parameter_name_bytes {
                return Err(Diagnostic::error(
                    "EXPR-PARAM-002",
                    "parameter name exceeds configured size limit",
                ));
            }
            match parameters.get(&parameter.name) {
                Some(existing) if existing != &node.ty => {
                    return Err(Diagnostic::error(
                        "EXPR-PARAM-003",
                        format!(
                            "parameter `{}` is used with more than one semantic type",
                            parameter.name
                        ),
                    ));
                }
                Some(_) => {}
                None => {
                    parameters.insert(Arc::clone(&parameter.name), node.ty.clone());
                }
            }
        }
        ExprKind::Unary { op, input } => {
            validate_node(input, limits, parameters)?;
            validate_unary(*op, input, node)?;
        }
        ExprKind::Binary { op, left, right } => {
            validate_node(left, limits, parameters)?;
            validate_node(right, limits, parameters)?;
            validate_binary(*op, left, right, node)?;
        }
        ExprKind::Membership {
            input, candidates, ..
        } => {
            validate_node(input, limits, parameters)?;
            for candidate in candidates {
                validate_node(candidate, limits, parameters)?;
                require_exact_type(&candidate.ty, &input.ty, "membership candidate")?;
            }
            if !input.ty.properties().equality {
                return Err(Diagnostic::error(
                    "EXPR-TYPE-010",
                    "membership requires a type with equality semantics",
                ));
            }
            require_truth(&node.ty, "membership result")?;
        }
        ExprKind::Conditional {
            condition,
            when_true,
            when_false,
        } => {
            validate_node(condition, limits, parameters)?;
            validate_node(when_true, limits, parameters)?;
            validate_node(when_false, limits, parameters)?;
            require_truth(&condition.ty, "conditional condition")?;
            require_exact_type(&when_true.ty, &when_false.ty, "conditional branches")?;
            require_exact_type(&node.ty, &when_true.ty, "conditional result")?;
        }
        ExprKind::FunctionCall {
            function,
            arguments,
        } => {
            for argument in arguments {
                validate_node(argument, limits, parameters)?;
            }
            let argument_types = arguments
                .iter()
                .map(|argument| argument.ty.clone())
                .collect::<Vec<_>>();
            function.validate(&argument_types, &node.ty)?;
        }
        ExprKind::Exists(_) => {
            require_truth(&node.ty, "existential result")?;
        }
    }
    Ok(())
}

fn validate_unary(op: UnaryOp, input: &ExprNode, output: &ExprNode) -> Result<()> {
    match op {
        UnaryOp::Not => require_truth(&input.ty, "logical NOT")?,
        UnaryOp::IsNull | UnaryOp::IsMissing | UnaryOp::IsPresent => {}
        UnaryOp::NullableLift if input.ty.is_nullable() => {
            return Err(Diagnostic::error(
                "EXPR-TYPE-015",
                "nullable lift requires a non-null semantic input type",
            ));
        }
        UnaryOp::NullableLift => {}
        UnaryOp::LosslessCast => validate_lossless_cast(&input.ty, &output.ty)?,
        UnaryOp::Negate | UnaryOp::Abs => require_negatable(&input.ty)?,
    }

    let expected = match op {
        UnaryOp::Not | UnaryOp::IsNull | UnaryOp::IsMissing | UnaryOp::IsPresent => {
            Truth::type_def()
        }
        UnaryOp::NullableLift => input.ty.clone().nullable(),
        UnaryOp::LosslessCast => output.ty.clone(),
        UnaryOp::Negate | UnaryOp::Abs => input.ty.clone(),
    };
    require_exact_type(&output.ty, &expected, "unary expression result")
}

fn validate_binary(
    op: BinaryOp,
    left: &ExprNode,
    right: &ExprNode,
    output: &ExprNode,
) -> Result<()> {
    match op {
        BinaryOp::Eq | BinaryOp::Ne => {
            require_same_input_types(left, right)?;
            if !left.ty.properties().equality {
                return Err(Diagnostic::error(
                    "EXPR-TYPE-003",
                    "type does not define equality semantics",
                ));
            }
            require_truth(&output.ty, "equality result")
        }
        BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
            require_same_input_types(left, right)?;
            if !left.ty.properties().ordering {
                return Err(Diagnostic::error(
                    "EXPR-TYPE-004",
                    "type does not define ordering semantics",
                ));
            }
            require_truth(&output.ty, "ordering result")
        }
        BinaryOp::And | BinaryOp::Or => {
            require_truth(&left.ty, "logical left operand")?;
            require_truth(&right.ty, "logical right operand")?;
            require_truth(&output.ty, "logical result")
        }
        BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div => {
            require_same_input_types(left, right)?;
            if !left.ty.properties().numeric {
                return Err(Diagnostic::error(
                    "EXPR-TYPE-005",
                    "type does not define numeric semantics",
                ));
            }
            require_numeric_representation(&left.ty)?;
            require_exact_type(&output.ty, &left.ty, "numeric result")
        }
        BinaryOp::Rem => {
            require_same_input_types(left, right)?;
            if !left.ty.properties().integral {
                return Err(Diagnostic::error(
                    "EXPR-TYPE-011",
                    "remainder requires integral numeric semantics",
                ));
            }
            require_integral_representation(&left.ty)?;
            require_exact_type(&output.ty, &left.ty, "remainder result")
        }
        BinaryOp::IsDistinctFrom | BinaryOp::IsNotDistinctFrom => {
            require_same_input_types(left, right)?;
            if !left.ty.properties().equality {
                return Err(Diagnostic::error(
                    "EXPR-TYPE-003",
                    "type does not define equality semantics",
                ));
            }
            require_truth(&output.ty, "null-safe equality result")
        }
        BinaryOp::Coalesce => validate_coalesce(left, right, output),
    }
}

pub(crate) fn validate_builtin_function(
    function: &FunctionDef,
    arguments: &[TypeDef],
    output: &TypeDef,
) -> Result<()> {
    match function.key().as_str() {
        "dol/text/to-lowercase"
        | "dol/text/to-uppercase"
        | "dol/text/trim"
        | "dol/text/trim-start"
        | "dol/text/trim-end" => {
            let input = unary_type_argument(arguments)?;
            require_string(input)?;
            require_exact_type(output, input, "text function result")
        }
        "dol/text/contains" | "dol/text/starts-with" | "dol/text/ends-with" => {
            let (left, right) = binary_type_arguments(arguments)?;
            require_exact_type(left, right, "text function inputs")?;
            require_string(left)?;
            require_truth(output, "text predicate result")
        }
        "dol/text/byte-length" | "dol/text/scalar-length" => {
            let input = unary_type_argument(arguments)?;
            require_string(input)?;
            let expected = preserve_nullability(u64::type_def(), input);
            require_exact_type(output, &expected, "text length result")
        }
        "dol/date/year" => {
            let input = unary_type_argument(arguments)?;
            require_builtin(input, &time::Date::type_def(), "date")?;
            let expected = preserve_nullability(i32::type_def(), input);
            require_exact_type(output, &expected, "date year result")
        }
        "dol/date/month" => {
            let input = unary_type_argument(arguments)?;
            require_builtin(input, &time::Date::type_def(), "date")?;
            let expected = preserve_nullability(time::Month::type_def(), input);
            require_exact_type(output, &expected, "date month result")
        }
        "dol/date/day" => {
            let input = unary_type_argument(arguments)?;
            require_builtin(input, &time::Date::type_def(), "date")?;
            let expected = preserve_nullability(u8::type_def(), input);
            require_exact_type(output, &expected, "date day result")
        }
        "dol/date/ordinal" => {
            let input = unary_type_argument(arguments)?;
            require_builtin(input, &time::Date::type_def(), "date")?;
            let expected = preserve_nullability(u16::type_def(), input);
            require_exact_type(output, &expected, "date ordinal result")
        }
        "dol/date/weekday" => {
            let input = unary_type_argument(arguments)?;
            require_builtin(input, &time::Date::type_def(), "date")?;
            let expected = preserve_nullability(time::Weekday::type_def(), input);
            require_exact_type(output, &expected, "date weekday result")
        }
        "dol/local-datetime/date" => {
            let input = unary_type_argument(arguments)?;
            require_builtin(
                input,
                &time::PrimitiveDateTime::type_def(),
                "local datetime",
            )?;
            let expected = preserve_nullability(time::Date::type_def(), input);
            require_exact_type(output, &expected, "local datetime date result")
        }
        "dol/local-datetime/time" => {
            let input = unary_type_argument(arguments)?;
            require_builtin(
                input,
                &time::PrimitiveDateTime::type_def(),
                "local datetime",
            )?;
            let expected = preserve_nullability(time::Time::type_def(), input);
            require_exact_type(output, &expected, "local datetime time result")
        }
        "dol/instant/unix-timestamp" => {
            let input = unary_type_argument(arguments)?;
            require_builtin(input, &time::OffsetDateTime::type_def(), "instant")?;
            let expected = preserve_nullability(i64::type_def(), input);
            require_exact_type(output, &expected, "instant timestamp result")
        }
        "dol/duration/whole-seconds" => {
            let input = unary_type_argument(arguments)?;
            require_builtin(input, &time::Duration::type_def(), "duration")?;
            let expected = preserve_nullability(i64::type_def(), input);
            require_exact_type(output, &expected, "duration seconds result")
        }
        key => Err(Diagnostic::error(
            "EXPR-FUNCTION-003",
            format!("no built-in validator exists for semantic function `{key}`"),
        )),
    }
}

fn unary_type_argument(arguments: &[TypeDef]) -> Result<&TypeDef> {
    if arguments.len() != 1 {
        return Err(Diagnostic::error(
            "EXPR-FUNCTION-001",
            "function requires exactly one argument",
        ));
    }
    arguments
        .first()
        .ok_or_else(|| Diagnostic::error("EXPR-FUNCTION-001", "function argument is missing"))
}

fn binary_type_arguments(arguments: &[TypeDef]) -> Result<(&TypeDef, &TypeDef)> {
    if arguments.len() != 2 {
        return Err(Diagnostic::error(
            "EXPR-FUNCTION-001",
            "function requires exactly two arguments",
        ));
    }
    let left = arguments
        .first()
        .ok_or_else(|| Diagnostic::error("EXPR-FUNCTION-001", "left argument is missing"))?;
    let right = arguments
        .get(1)
        .ok_or_else(|| Diagnostic::error("EXPR-FUNCTION-001", "right argument is missing"))?;
    Ok((left, right))
}

fn preserve_nullability(mut ty: TypeDef, input: &TypeDef) -> TypeDef {
    if input.is_nullable() {
        ty = ty.nullable();
    }
    ty
}

fn require_builtin(actual: &TypeDef, expected: &TypeDef, name: &str) -> Result<()> {
    if actual.clone().non_null() == *expected {
        Ok(())
    } else {
        Err(Diagnostic::error(
            "EXPR-FUNCTION-002",
            format!("function requires the built-in DOL {name} type"),
        ))
    }
}

fn require_same_input_types(left: &ExprNode, right: &ExprNode) -> Result<()> {
    if left.ty == right.ty {
        Ok(())
    } else {
        Err(Diagnostic::error(
            "EXPR-TYPE-006",
            "binary operands must have identical semantic types",
        ))
    }
}

fn require_truth(ty: &TypeDef, context: &str) -> Result<()> {
    require_exact_type(ty, &Truth::type_def(), context)
}

fn require_string(ty: &TypeDef) -> Result<()> {
    if matches!(ty.shape(), TypeShape::Scalar(ScalarRepr::String))
        && ty.key() == String::type_def().key()
    {
        Ok(())
    } else {
        Err(Diagnostic::error(
            "EXPR-TYPE-007",
            "operation requires the built-in DOL string type",
        ))
    }
}

fn require_numeric_representation(ty: &TypeDef) -> Result<()> {
    if matches!(
        ty.shape(),
        TypeShape::Scalar(
            ScalarRepr::Int { .. }
                | ScalarRepr::UInt { .. }
                | ScalarRepr::Float32
                | ScalarRepr::Float64
                | ScalarRepr::Decimal
        )
    ) {
        Ok(())
    } else {
        Err(Diagnostic::error(
            "EXPR-TYPE-009",
            "numeric expression requires a canonical numeric representation",
        ))
    }
}

fn require_exact_type(actual: &TypeDef, expected: &TypeDef, context: &str) -> Result<()> {
    if actual == expected {
        Ok(())
    } else {
        Err(Diagnostic::error(
            "EXPR-TYPE-008",
            format!("{context} has an unexpected semantic type"),
        ))
    }
}

fn require_integral_representation(ty: &TypeDef) -> Result<()> {
    if matches!(
        ty.shape(),
        TypeShape::Scalar(ScalarRepr::Int { .. } | ScalarRepr::UInt { .. })
    ) {
        Ok(())
    } else {
        Err(Diagnostic::error(
            "EXPR-TYPE-012",
            "integral expression requires a canonical integer representation",
        ))
    }
}

fn require_negatable(ty: &TypeDef) -> Result<()> {
    if matches!(
        ty.shape(),
        TypeShape::Scalar(
            ScalarRepr::Int { .. }
                | ScalarRepr::Float32
                | ScalarRepr::Float64
                | ScalarRepr::Decimal
        )
    ) && ty.properties().numeric
    {
        Ok(())
    } else {
        Err(Diagnostic::error(
            "EXPR-TYPE-013",
            "signed numeric unary operation requires signed, floating-point, or decimal semantics",
        ))
    }
}

fn validate_coalesce(left: &ExprNode, right: &ExprNode, output: &ExprNode) -> Result<()> {
    if left.ty == right.ty {
        return require_exact_type(&output.ty, &left.ty, "coalesce result");
    }

    let left_non_null = left.ty.clone().non_null();
    if left.ty.is_nullable() && left_non_null == right.ty {
        require_exact_type(&output.ty, &right.ty, "coalesce result")
    } else {
        Err(Diagnostic::error(
            "EXPR-TYPE-014",
            "coalesce fallback must match the nullable input's semantic type",
        ))
    }
}
