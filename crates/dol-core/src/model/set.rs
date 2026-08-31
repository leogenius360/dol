use std::collections::{BTreeMap, BTreeSet};

use crate::diagnostic::{Diagnostic, Result};
use crate::types::validate_type_universe;

use super::{Cardinality, FieldKey, ModelDef, ModelKey};

/// Immutable validated collection of mutually resolvable models.
#[derive(Debug, Clone)]
pub struct ModelSet {
    models: BTreeMap<ModelKey, ModelDef>,
}

impl ModelSet {
    /// Validates and creates a model set.
    pub fn new(models: impl IntoIterator<Item = ModelDef>) -> Result<Self> {
        let mut map = BTreeMap::new();
        for model in models {
            if map.insert(model.key().clone(), model).is_some() {
                return Err(Diagnostic::error("MODELSET-001", "duplicate model key"));
            }
        }

        let set = Self { models: map };
        set.validate_type_identities()?;
        set.validate_cross_model()?;
        Ok(set)
    }

    /// Returns a model by stable key.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&ModelDef> {
        self.models.get(key)
    }

    /// Iterates models in canonical key order.
    pub fn iter(&self) -> impl Iterator<Item = &ModelDef> {
        self.models.values()
    }

    fn validate_type_identities(&self) -> Result<()> {
        validate_type_universe(
            self.models
                .values()
                .flat_map(|model| model.fields())
                .map(|field| field.ty()),
        )
    }

    fn validate_cross_model(&self) -> Result<()> {
        for source in self.models.values() {
            self.validate_relations(source)?;
            self.validate_references(source)?;
        }
        Ok(())
    }

    fn validate_relations(&self, source: &ModelDef) -> Result<()> {
        for relation in source.relations() {
            let target = self.models.get(relation.target_model()).ok_or_else(|| {
                Diagnostic::error(
                    "RELATION-101",
                    format!(
                        "relation `{}` targets unknown model `{}`",
                        relation.name(),
                        relation.target_model().as_str()
                    ),
                )
            })?;

            for pair in relation.fields() {
                let left = source.field(pair.source().as_str()).ok_or_else(|| {
                    Diagnostic::error("RELATION-102", "relation source field missing after freeze")
                })?;
                let right = target.field(pair.target().as_str()).ok_or_else(|| {
                    Diagnostic::error(
                        "RELATION-103",
                        format!(
                            "relation `{}` targets unknown field `{}`",
                            relation.name(),
                            pair.target().as_str()
                        ),
                    )
                })?;

                if !left.ty().same_value_type(right.ty()) {
                    return Err(Diagnostic::error(
                        "RELATION-104",
                        format!("relation `{}` maps incompatible types", relation.name()),
                    ));
                }
            }

            if matches!(
                relation.cardinality(),
                Cardinality::One | Cardinality::OptionalOne
            ) {
                let target_fields: BTreeSet<_> = relation
                    .fields()
                    .iter()
                    .map(|pair| pair.target().clone())
                    .collect();
                if !is_unique(target, &target_fields) {
                    return Err(Diagnostic::error(
                        "RELATION-105",
                        format!(
                            "to-one relation `{}` must target an identity or unique field tuple",
                            relation.name()
                        ),
                    ));
                }
            }
        }

        Ok(())
    }

    fn validate_references(&self, source: &ModelDef) -> Result<()> {
        for reference in source.references() {
            let target = self.models.get(reference.target_model()).ok_or_else(|| {
                Diagnostic::error(
                    "CONSTRAINT-101",
                    format!(
                        "reference targets unknown model `{}`",
                        reference.target_model().as_str()
                    ),
                )
            })?;

            for (source_key, target_key) in reference
                .source_fields()
                .iter()
                .zip(reference.target_fields())
            {
                let left = source.field(source_key.as_str()).ok_or_else(|| {
                    Diagnostic::error(
                        "CONSTRAINT-102",
                        "reference source field missing after freeze",
                    )
                })?;
                let right = target.field(target_key.as_str()).ok_or_else(|| {
                    Diagnostic::error(
                        "CONSTRAINT-103",
                        format!("reference targets unknown field `{}`", target_key.as_str()),
                    )
                })?;

                if !left.ty().same_value_type(right.ty()) {
                    return Err(Diagnostic::error(
                        "CONSTRAINT-104",
                        "reference field types must match exactly",
                    ));
                }
            }

            let target_fields: BTreeSet<_> = reference.target_fields().iter().cloned().collect();
            if !is_unique(target, &target_fields) {
                return Err(Diagnostic::error(
                    "CONSTRAINT-105",
                    "reference target fields must form identity or unique tuple",
                ));
            }
        }

        Ok(())
    }
}

fn is_unique(model: &ModelDef, fields: &BTreeSet<FieldKey>) -> bool {
    if let Some(identity) = model.identity()
        && identity.fields().iter().cloned().collect::<BTreeSet<_>>() == *fields
    {
        return true;
    }

    model
        .unique_constraints()
        .iter()
        .any(|unique| unique.fields().iter().cloned().collect::<BTreeSet<_>>() == *fields)
}
