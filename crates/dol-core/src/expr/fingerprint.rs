use crate::diagnostic::Result;
use crate::fingerprint::{CanonicalHasher, Fingerprint, hash_datum, hash_type_def};

use super::function::hash_function_def;
use super::node::{BinaryOp, ExprKind, ExprNode, UnaryOp};

pub(crate) fn fingerprint_node(node: &ExprNode) -> Result<Fingerprint> {
    let mut hasher = CanonicalHasher::new(b"expression/v4");
    hash_node(&mut hasher, node)?;
    Ok(hasher.finish())
}

fn hash_node(hasher: &mut CanonicalHasher, node: &ExprNode) -> Result<()> {
    hash_type_def(hasher, &node.ty);
    match &node.kind {
        ExprKind::Field(field) => {
            hasher.u8(0);
            hasher.str(field.model.as_str());
            hasher.str(field.key.as_str());
        }
        ExprKind::Literal(value) => {
            hasher.u8(1);
            hash_datum(hasher, value);
        }
        ExprKind::Parameter(parameter) => {
            hasher.u8(2);
            hasher.str(&parameter.name);
        }
        ExprKind::Unary { op, input } => {
            hasher.u8(3);
            hasher.u8(unary_tag(*op));
            hash_node(hasher, input)?;
        }
        ExprKind::Binary { op, left, right } => {
            hasher.u8(4);
            hasher.u8(binary_tag(*op));
            hash_node(hasher, left)?;
            hash_node(hasher, right)?;
        }
        ExprKind::Membership {
            input,
            candidates,
            negate,
        } => {
            hasher.u8(5);
            hasher.u8(u8::from(*negate));
            hash_node(hasher, input)?;
            hasher.u64(candidates.len() as u64);
            for candidate in candidates {
                hash_node(hasher, candidate)?;
            }
        }
        ExprKind::Conditional {
            condition,
            when_true,
            when_false,
        } => {
            hasher.u8(6);
            hash_node(hasher, condition)?;
            hash_node(hasher, when_true)?;
            hash_node(hasher, when_false)?;
        }
        ExprKind::FunctionCall {
            function,
            arguments,
        } => {
            hasher.u8(7);
            hash_function_def(hasher, function.definition());
            hasher.u64(arguments.len() as u64);
            for argument in arguments {
                hash_node(hasher, argument)?;
            }
        }
        ExprKind::Exists(exists) => {
            hasher.u8(8);
            let subquery = exists.subquery_fingerprint()?;
            hasher.bytes(subquery.as_bytes());
        }
    }
    Ok(())
}

fn unary_tag(op: UnaryOp) -> u8 {
    match op {
        UnaryOp::Not => 0,
        UnaryOp::IsNull => 1,
        UnaryOp::IsMissing => 2,
        UnaryOp::IsPresent => 3,
        UnaryOp::NullableLift => 4,
        UnaryOp::LosslessCast => 5,
        UnaryOp::Negate => 6,
        UnaryOp::Abs => 7,
    }
}

fn binary_tag(op: BinaryOp) -> u8 {
    match op {
        BinaryOp::Eq => 0,
        BinaryOp::Ne => 1,
        BinaryOp::Lt => 2,
        BinaryOp::Le => 3,
        BinaryOp::Gt => 4,
        BinaryOp::Ge => 5,
        BinaryOp::And => 6,
        BinaryOp::Or => 7,
        BinaryOp::Add => 8,
        BinaryOp::Sub => 9,
        BinaryOp::Mul => 10,
        BinaryOp::Div => 11,
        BinaryOp::Rem => 12,
        BinaryOp::IsDistinctFrom => 13,
        BinaryOp::IsNotDistinctFrom => 14,
        BinaryOp::Coalesce => 15,
    }
}
