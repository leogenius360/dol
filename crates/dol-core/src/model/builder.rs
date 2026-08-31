use std::collections::{BTreeMap, BTreeSet};

use crate::diagnostic::{Diagnostic, Result};
use crate::limits::DefinitionLimits;
use crate::types::{Nullability, TypeDef, type_node_count, validate_type, validate_type_universe};

use super::{
    FieldDef, FieldKey, FieldSlot, IdentityDef, ModelDef, ModelKey, ModelParts, Presence,
    ReferenceDef, RelationDef, UniqueDef,
};

#[derive(Debug, Clone)]
struct FieldSpec {
    key: FieldKey,
    name: String,
    ty: TypeDef,
    presence: Presence,
}

/// Mutable model specification that freezes into an immutable `ModelDef`.
#[derive(Debug, Clone)]
pub struct ModelBuilder {
    key: ModelKey,
    name: String,
    fields: Vec<FieldSpec>,
    identity: Option<Vec<FieldKey>>,
    identity_redeclared: bool,
    unique: Vec<Vec<FieldKey>>,
    relations: Vec<RelationDef>,
    references: Vec<ReferenceDef>,
    limits: DefinitionLimits,
}

impl ModelBuilder {
    /// Creates a model whose stable key equals its current name.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();
        Self::with_key(name.clone(), name)
    }

    /// Creates a model with explicit stable semantic key and current name.
    #[must_use]
    pub fn with_key(key: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            key: ModelKey::new(key),
            name: name.into(),
            fields: Vec::new(),
            identity: None,
            identity_redeclared: false,
            unique: Vec::new(),
            relations: Vec::new(),
            references: Vec::new(),
            limits: DefinitionLimits::default(),
        }
    }

    /// Overrides definition limits.
    #[must_use]
    pub fn limits(mut self, limits: DefinitionLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Adds a required field whose stable key equals its name.
    #[must_use]
    pub fn field<T: crate::types::DataType>(self, name: impl Into<String>) -> Self {
        let name = name.into();
        self.field_def(name.clone(), name, T::type_def(), Presence::Required)
    }

    /// Adds a required field through an explicit semantic binding.
    #[must_use]
    pub fn field_with<T, B>(self, name: impl Into<String>) -> Self
    where
        B: crate::binding::SemanticBinding<T>,
    {
        let name = name.into();
        self.field_def(name.clone(), name, B::type_def(), Presence::Required)
    }

    /// Adds a field from a runtime semantic type definition.
    #[must_use]
    pub fn field_def(
        mut self,
        key: impl Into<String>,
        name: impl Into<String>,
        mut ty: TypeDef,
        presence: Presence,
    ) -> Self {
        ty.canonicalize();
        self.fields.push(FieldSpec {
            key: FieldKey::new(key),
            name: name.into(),
            ty,
            presence,
        });
        self
    }

    /// Adds an optional (missing allowed) field whose stable key equals its name.
    #[must_use]
    pub fn optional_field<T: crate::types::DataType>(self, name: impl Into<String>) -> Self {
        let name = name.into();
        self.field_def(name.clone(), name, T::type_def(), Presence::Optional)
    }

    /// Adds an optional field through an explicit semantic binding.
    #[must_use]
    pub fn optional_field_with<T, B>(self, name: impl Into<String>) -> Self
    where
        B: crate::binding::SemanticBinding<T>,
    {
        let name = name.into();
        self.field_def(name.clone(), name, B::type_def(), Presence::Optional)
    }

    /// Declares canonical model identity.
    #[must_use]
    pub fn identity<I, S>(mut self, fields: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let identity = fields
            .into_iter()
            .map(|field| FieldKey::new(field.into()))
            .collect();
        if self.identity.replace(identity).is_some() {
            self.identity_redeclared = true;
        }
        self
    }

    /// Declares a unique field tuple.
    #[must_use]
    pub fn unique<I, S>(mut self, fields: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.unique.push(
            fields
                .into_iter()
                .map(|field| FieldKey::new(field.into()))
                .collect(),
        );
        self
    }

    /// Adds a logical relation.
    #[must_use]
    pub fn relation(mut self, relation: RelationDef) -> Self {
        self.relations.push(relation);
        self
    }

    /// Adds a referential integrity constraint.
    #[must_use]
    pub fn reference<SI, S, TI, T>(
        mut self,
        source_fields: SI,
        target_model: impl Into<String>,
        target_fields: TI,
    ) -> Self
    where
        SI: IntoIterator<Item = S>,
        S: Into<String>,
        TI: IntoIterator<Item = T>,
        T: Into<String>,
    {
        self.references.push(ReferenceDef::new(
            source_fields
                .into_iter()
                .map(|field| FieldKey::new(field.into()))
                .collect(),
            ModelKey::new(target_model),
            target_fields
                .into_iter()
                .map(|field| FieldKey::new(field.into()))
                .collect(),
        ));
        self
    }

    /// Validates, canonicalizes, assigns slots, fingerprints, and freezes the model.
    pub fn freeze(mut self) -> Result<ModelDef> {
        validate_name("MODEL-001", "model key", self.key.as_str(), self.limits)?;
        validate_name("MODEL-002", "model name", &self.name, self.limits)?;

        if self.fields.len() > self.limits.max_fields {
            return Err(Diagnostic::error(
                "LIMIT-001",
                format!(
                    "model has {} fields; limit is {}",
                    self.fields.len(),
                    self.limits.max_fields
                ),
            ));
        }
        if self.relations.len() > self.limits.max_relations {
            return Err(Diagnostic::error(
                "LIMIT-002",
                "model exceeds relation limit",
            ));
        }

        let identity_count = if self.identity.is_some() { 1 } else { 0 };
        let constraint_count = self.unique.len() + self.references.len() + identity_count;
        if constraint_count > self.limits.max_constraints {
            return Err(Diagnostic::error(
                "LIMIT-003",
                "model exceeds constraint limit",
            ));
        }

        let type_nodes = self.fields.iter().fold(0usize, |total, field| {
            total.saturating_add(type_node_count(&field.ty))
        });
        if type_nodes > self.limits.max_type_nodes {
            return Err(Diagnostic::error(
                "LIMIT-005",
                format!(
                    "model has {type_nodes} type nodes; limit is {}",
                    self.limits.max_type_nodes
                ),
            ));
        }

        if self.identity_redeclared {
            return Err(Diagnostic::error(
                "CONSTRAINT-008",
                "model identity may only be declared once",
            ));
        }

        self.validate_fields()?;
        let by_key: BTreeMap<_, _> = self
            .fields
            .iter()
            .map(|field| (field.key.clone(), field))
            .collect();
        self.validate_constraints(&by_key)?;
        self.validate_relations(&by_key)?;
        self.validate_references(&by_key)?;

        self.canonicalize();
        self.into_model_def()
    }

    fn validate_fields(&self) -> Result<()> {
        let mut keys = BTreeSet::new();
        let mut names = BTreeSet::new();

        for field in &self.fields {
            validate_name("FIELD-001", "field key", field.key.as_str(), self.limits)?;
            validate_name("FIELD-002", "field name", &field.name, self.limits)?;
            if !keys.insert(field.key.clone()) {
                return Err(Diagnostic::error(
                    "FIELD-003",
                    format!("duplicate field key `{}`", field.key.as_str()),
                ));
            }
            if !names.insert(field.name.clone()) {
                return Err(Diagnostic::error(
                    "FIELD-004",
                    format!("duplicate field name `{}`", field.name),
                ));
            }
            validate_type(&field.ty, self.limits)?;
        }

        validate_type_universe(self.fields.iter().map(|field| &field.ty))
    }

    fn validate_constraints(&self, by_key: &BTreeMap<FieldKey, &FieldSpec>) -> Result<()> {
        if let Some(identity) = &self.identity {
            validate_field_tuple("CONSTRAINT-001", identity, by_key)?;
            for key in identity {
                let field = by_key[key];
                if field.presence != Presence::Required
                    || field.ty.nullability() != Nullability::NonNull
                {
                    return Err(Diagnostic::error(
                        "CONSTRAINT-002",
                        format!(
                            "identity field `{}` must be required and non-null",
                            key.as_str()
                        ),
                    ));
                }
                if !field.ty.properties().equality || !field.ty.properties().keyable {
                    return Err(Diagnostic::error(
                        "CONSTRAINT-006",
                        format!(
                            "identity field `{}` must support stable key equality",
                            key.as_str()
                        ),
                    ));
                }
            }
        }

        for unique in &self.unique {
            validate_field_tuple("CONSTRAINT-003", unique, by_key)?;
            for key in unique {
                if !by_key[key].ty.properties().equality || !by_key[key].ty.properties().keyable {
                    return Err(Diagnostic::error(
                        "CONSTRAINT-007",
                        format!(
                            "unique field `{}` must support stable key equality",
                            key.as_str()
                        ),
                    ));
                }
            }
        }

        Ok(())
    }

    fn validate_relations(&self, by_key: &BTreeMap<FieldKey, &FieldSpec>) -> Result<()> {
        let mut relation_keys = BTreeSet::new();
        let mut relation_names = BTreeSet::new();

        for relation in &self.relations {
            validate_name(
                "RELATION-003",
                "relation key",
                relation.key().as_str(),
                self.limits,
            )?;
            validate_name(
                "RELATION-004",
                "relation name",
                relation.name(),
                self.limits,
            )?;
            if !relation_keys.insert(relation.key().clone()) {
                return Err(Diagnostic::error(
                    "RELATION-005",
                    format!("duplicate relation key `{}`", relation.key().as_str()),
                ));
            }
            if !relation_names.insert(relation.name().to_owned()) {
                return Err(Diagnostic::error(
                    "RELATION-006",
                    format!("duplicate relation name `{}`", relation.name()),
                ));
            }
            validate_name(
                "RELATION-008",
                "relation target model key",
                relation.target_model().as_str(),
                self.limits,
            )?;
            if relation.fields().is_empty() {
                return Err(Diagnostic::error(
                    "RELATION-001",
                    "relation must map at least one field",
                ));
            }

            let mut sources = BTreeSet::new();
            let mut targets = BTreeSet::new();
            for pair in relation.fields() {
                validate_name(
                    "RELATION-009",
                    "relation target field key",
                    pair.target().as_str(),
                    self.limits,
                )?;
                if !by_key.contains_key(pair.source()) {
                    return Err(Diagnostic::error(
                        "RELATION-002",
                        format!("unknown source field `{}`", pair.source().as_str()),
                    ));
                }
                if !sources.insert(pair.source().clone()) || !targets.insert(pair.target().clone())
                {
                    return Err(Diagnostic::error(
                        "RELATION-007",
                        "relation field mappings must not repeat source or target fields",
                    ));
                }
            }
        }

        Ok(())
    }

    fn validate_references(&self, by_key: &BTreeMap<FieldKey, &FieldSpec>) -> Result<()> {
        for reference in &self.references {
            validate_name(
                "CONSTRAINT-010",
                "reference target model key",
                reference.target_model().as_str(),
                self.limits,
            )?;
            if reference.source_fields().len() != reference.target_fields().len()
                || reference.source_fields().is_empty()
            {
                return Err(Diagnostic::error(
                    "CONSTRAINT-004",
                    "reference source/target field tuples must be non-empty and equal in width",
                ));
            }
            validate_field_tuple("CONSTRAINT-005", reference.source_fields(), by_key)?;

            let mut target_fields = BTreeSet::new();
            for field in reference.target_fields() {
                validate_name(
                    "CONSTRAINT-011",
                    "reference target field key",
                    field.as_str(),
                    self.limits,
                )?;
                if !target_fields.insert(field) {
                    return Err(Diagnostic::error(
                        "CONSTRAINT-009",
                        format!("duplicate target field `{}` in reference", field.as_str()),
                    ));
                }
            }
        }

        Ok(())
    }

    fn canonicalize(&mut self) {
        self.fields.sort_by(|left, right| left.key.cmp(&right.key));
        self.relations
            .sort_by(|left, right| left.key().cmp(right.key()));

        for fields in &mut self.unique {
            fields.sort();
        }
        self.unique.sort();
        self.unique.dedup();

        self.references.sort_by(|left, right| {
            (
                left.target_model(),
                left.source_fields(),
                left.target_fields(),
            )
                .cmp(&(
                    right.target_model(),
                    right.source_fields(),
                    right.target_fields(),
                ))
        });
        self.references.dedup();
    }

    fn into_model_def(self) -> Result<ModelDef> {
        let mut fields = Vec::with_capacity(self.fields.len());
        for (index, field) in self.fields.into_iter().enumerate() {
            let index = u32::try_from(index).map_err(|_| {
                Diagnostic::error("LIMIT-004", "model field count exceeds slot address space")
            })?;
            fields.push(FieldDef::new(
                field.key,
                field.name,
                FieldSlot::new(index),
                field.ty,
                field.presence,
            ));
        }

        let identity = self.identity.map(IdentityDef::new);
        let unique = self.unique.into_iter().map(UniqueDef::new).collect();
        Ok(ModelDef::from_parts(ModelParts {
            key: self.key,
            name: self.name,
            fields,
            identity,
            unique,
            relations: self.relations,
            references: self.references,
        }))
    }
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

fn validate_field_tuple(
    code: &'static str,
    fields: &[FieldKey],
    known: &BTreeMap<FieldKey, &FieldSpec>,
) -> Result<()> {
    if fields.is_empty() {
        return Err(Diagnostic::error(code, "field tuple must not be empty"));
    }

    let mut seen = BTreeSet::new();
    for field in fields {
        if !known.contains_key(field) {
            return Err(Diagnostic::error(
                code,
                format!("unknown field `{}`", field.as_str()),
            ));
        }
        if !seen.insert(field) {
            return Err(Diagnostic::error(
                code,
                format!("duplicate field `{}` in constraint", field.as_str()),
            ));
        }
    }

    Ok(())
}
