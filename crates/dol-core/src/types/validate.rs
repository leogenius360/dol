use std::collections::{BTreeMap, BTreeSet};

use crate::diagnostic::{Diagnostic, Result};
use crate::limits::DefinitionLimits;

use super::{ScalarRepr, TypeDef, TypeKey, TypeParameter, TypeShape};

/// Validates a semantic type definition independently of any model.
pub fn validate_type(ty: &TypeDef, limits: DefinitionLimits) -> Result<()> {
    validate_structure_limits(ty, limits)?;
    validate_type_at_depth(ty, 0, limits)
}

fn validate_structure_limits(root: &TypeDef, limits: DefinitionLimits) -> Result<()> {
    let mut stack = vec![(root, 0_usize)];
    let mut nodes = 0_usize;
    while let Some((ty, depth)) = stack.pop() {
        if depth > limits.max_type_depth {
            return Err(Diagnostic::error(
                "TYPE-001",
                "type nesting exceeds configured depth",
            ));
        }
        nodes = nodes.saturating_add(1);
        if nodes > limits.max_type_nodes {
            return Err(Diagnostic::error(
                "TYPE-010",
                format!("type has more than {} nodes", limits.max_type_nodes),
            ));
        }
        let child_depth = depth.saturating_add(1);
        match ty.shape() {
            TypeShape::Scalar(_) => {}
            TypeShape::List(element) => stack.push((element, child_depth)),
            TypeShape::Map { key, value } => {
                stack.push((value, child_depth));
                stack.push((key, child_depth));
            }
            TypeShape::Tuple(elements) => {
                stack.extend(elements.iter().rev().map(|element| (element, child_depth)));
            }
            TypeShape::Record(fields) => {
                stack.extend(fields.iter().rev().map(|field| (field.ty(), child_depth)));
            }
        }
    }
    Ok(())
}

/// Validates that each semantic type identity/version has one value definition.
pub fn validate_type_universe<'a>(types: impl IntoIterator<Item = &'a TypeDef>) -> Result<()> {
    let mut known = BTreeMap::<(TypeKey, u32), TypeDef>::new();
    for ty in types {
        register_type_identity(&mut known, ty)?;
    }
    Ok(())
}

fn register_type_identity(
    known: &mut BTreeMap<(TypeKey, u32), TypeDef>,
    ty: &TypeDef,
) -> Result<()> {
    let key = (ty.key().clone(), ty.version());
    if let Some(existing) = known.get(&key) {
        if !existing.same_value_type(ty) {
            return Err(Diagnostic::error(
                "TYPE-101",
                format!(
                    "semantic type `{}@{}` has conflicting definitions",
                    ty.key().as_str(),
                    ty.version()
                ),
            ));
        }
    } else {
        known.insert(key, ty.clone().non_null());
    }

    match ty.shape() {
        TypeShape::Scalar(_) => {}
        TypeShape::List(element) => register_type_identity(known, element)?,
        TypeShape::Map { key, value } => {
            register_type_identity(known, key)?;
            register_type_identity(known, value)?;
        }
        TypeShape::Tuple(elements) => {
            for element in elements {
                register_type_identity(known, element)?;
            }
        }
        TypeShape::Record(fields) => {
            for field in fields {
                register_type_identity(known, field.ty())?;
            }
        }
    }

    Ok(())
}

pub(crate) fn type_node_count(ty: &TypeDef) -> usize {
    let mut stack = vec![ty];
    let mut nodes = 0_usize;
    while let Some(ty) = stack.pop() {
        nodes = nodes.saturating_add(1);
        match ty.shape() {
            TypeShape::Scalar(_) => {}
            TypeShape::List(element) => stack.push(element),
            TypeShape::Map { key, value } => {
                stack.push(value);
                stack.push(key);
            }
            TypeShape::Tuple(elements) => stack.extend(elements),
            TypeShape::Record(fields) => stack.extend(fields.iter().map(super::RecordField::ty)),
        }
    }
    nodes
}

fn validate_type_at_depth(ty: &TypeDef, depth: usize, limits: DefinitionLimits) -> Result<()> {
    if depth > limits.max_type_depth {
        return Err(Diagnostic::error(
            "TYPE-001",
            "type nesting exceeds configured depth",
        ));
    }
    validate_type_key(ty.key().as_str(), limits)?;
    if ty.version() == 0 {
        return Err(Diagnostic::error(
            "TYPE-007",
            "semantic type version must be greater than zero",
        ));
    }

    for (name, value) in ty.parameters() {
        validate_name("TYPE-011", "type parameter name", name, limits)?;
        if let TypeParameter::String(value) = value
            && value.len() > limits.max_name_bytes
        {
            return Err(Diagnostic::error(
                "TYPE-012",
                format!(
                    "type parameter `{name}` exceeds {} bytes",
                    limits.max_name_bytes
                ),
            ));
        }
    }

    match ty.shape() {
        TypeShape::Scalar(ScalarRepr::Int { bits } | ScalarRepr::UInt { bits })
            if !matches!(bits, 8 | 16 | 32 | 64 | 128) =>
        {
            return Err(Diagnostic::error(
                "TYPE-008",
                "integer representation width must be 8, 16, 32, 64, or 128 bits",
            ));
        }
        TypeShape::Scalar(_) => {}
        TypeShape::List(element) => validate_type_at_depth(element, depth + 1, limits)?,
        TypeShape::Map { key, value } => {
            validate_type_at_depth(key, depth + 1, limits)?;
            validate_type_at_depth(value, depth + 1, limits)?;
            if key.is_nullable() || !key.properties().equality || !key.properties().keyable {
                return Err(Diagnostic::error(
                    "TYPE-009",
                    "map keys must be non-null, equality-capable, and keyable",
                ));
            }
        }
        TypeShape::Tuple(elements) => {
            if elements.len() > limits.max_fields {
                return Err(Diagnostic::error("TYPE-003", "tuple exceeds width limit"));
            }
            for element in elements {
                validate_type_at_depth(element, depth + 1, limits)?;
            }
        }
        TypeShape::Record(fields) => {
            if fields.len() > limits.max_fields {
                return Err(Diagnostic::error("TYPE-004", "record exceeds width limit"));
            }

            let mut names = BTreeSet::new();
            for field in fields {
                validate_name("TYPE-005", "record field name", field.name(), limits)?;
                if !names.insert(field.name()) {
                    return Err(Diagnostic::error(
                        "TYPE-006",
                        format!("duplicate record field `{}`", field.name()),
                    ));
                }
                validate_type_at_depth(field.ty(), depth + 1, limits)?;
            }
        }
    }

    Ok(())
}

fn validate_type_key(value: &str, limits: DefinitionLimits) -> Result<()> {
    validate_name("TYPE-002", "type key", value, limits)?;
    if value.starts_with('/')
        || value.ends_with('/')
        || value.contains("//")
        || !value.contains('/')
        || value.chars().any(char::is_control)
        || value.chars().any(char::is_whitespace)
    {
        return Err(Diagnostic::error(
            "TYPE-013",
            "semantic type key must use a non-empty `namespace/name` form without whitespace",
        ));
    }
    Ok(())
}

fn validate_name(
    code: &'static str,
    kind: &str,
    value: &str,
    limits: DefinitionLimits,
) -> Result<()> {
    if value.is_empty() {
        return Err(Diagnostic::error(code, format!("{kind} must not be empty")));
    }
    if value.len() > limits.max_name_bytes {
        return Err(Diagnostic::error(
            code,
            format!("{kind} exceeds {} bytes", limits.max_name_bytes),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::limits::DefinitionLimits;
    use crate::types::{ScalarRepr, TypeDef, TypeShape, validate_type};

    #[test]
    fn depth_is_rejected_before_recursive_semantic_validation() {
        let mut ty = TypeDef::scalar("test/leaf", 1, ScalarRepr::Bool);
        for index in 0..2_048 {
            ty = TypeDef::shaped(
                format!("test/list-{index}"),
                1,
                TypeShape::List(Box::new(ty)),
            );
        }
        let error = validate_type(
            &ty,
            DefinitionLimits {
                max_type_depth: 32,
                max_type_nodes: 64,
                ..DefinitionLimits::default()
            },
        )
        .unwrap_err();
        assert!(matches!(error.code(), "TYPE-001" | "TYPE-010"));
    }
}
