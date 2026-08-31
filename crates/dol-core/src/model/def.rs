use std::collections::BTreeMap;

use crate::fingerprint::{CanonicalHasher, Fingerprint, hash_type_def};
use crate::types::Presence;

use super::{FieldDef, FieldKey, IdentityDef, ModelKey, ReferenceDef, RelationDef, UniqueDef};

/// Validated model components used to construct an immutable `ModelDef`.
pub(crate) struct ModelParts {
    pub(crate) key: ModelKey,
    pub(crate) name: String,
    pub(crate) fields: Vec<FieldDef>,
    pub(crate) identity: Option<IdentityDef>,
    pub(crate) unique: Vec<UniqueDef>,
    pub(crate) relations: Vec<RelationDef>,
    pub(crate) references: Vec<ReferenceDef>,
}

/// Immutable canonical logical model definition.
#[derive(Debug, Clone)]
pub struct ModelDef {
    key: ModelKey,
    name: String,
    fields: Vec<FieldDef>,
    fields_by_key: BTreeMap<FieldKey, usize>,
    fields_by_name: BTreeMap<String, usize>,
    identity: Option<IdentityDef>,
    unique: Vec<UniqueDef>,
    relations: Vec<RelationDef>,
    references: Vec<ReferenceDef>,
    fingerprint: Fingerprint,
}

impl ModelDef {
    pub(crate) fn from_parts(parts: ModelParts) -> Self {
        let fields_by_key = parts
            .fields
            .iter()
            .enumerate()
            .map(|(index, field)| (field.key().clone(), index))
            .collect();
        let fields_by_name = parts
            .fields
            .iter()
            .enumerate()
            .map(|(index, field)| (field.name().to_owned(), index))
            .collect();
        let fingerprint = fingerprint_model(&parts);

        Self {
            key: parts.key,
            name: parts.name,
            fields: parts.fields,
            fields_by_key,
            fields_by_name,
            identity: parts.identity,
            unique: parts.unique,
            relations: parts.relations,
            references: parts.references,
            fingerprint,
        }
    }

    /// Stable semantic model lineage key.
    #[must_use]
    pub const fn key(&self) -> &ModelKey {
        &self.key
    }

    /// Current logical model name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Canonically ordered fields.
    #[must_use]
    pub fn fields(&self) -> &[FieldDef] {
        &self.fields
    }

    /// Returns a field by stable key.
    #[must_use]
    pub fn field(&self, key: &str) -> Option<&FieldDef> {
        self.fields_by_key
            .get(key)
            .map(|index| &self.fields[*index])
    }

    /// Returns a field by its current logical name.
    #[must_use]
    pub fn field_named(&self, name: &str) -> Option<&FieldDef> {
        self.fields_by_name
            .get(name)
            .map(|index| &self.fields[*index])
    }

    /// Canonical identity constraint.
    #[must_use]
    pub const fn identity(&self) -> Option<&IdentityDef> {
        self.identity.as_ref()
    }

    /// Unique constraints.
    #[must_use]
    pub fn unique_constraints(&self) -> &[UniqueDef] {
        &self.unique
    }

    /// Relations.
    #[must_use]
    pub fn relations(&self) -> &[RelationDef] {
        &self.relations
    }

    /// Referential integrity constraints.
    #[must_use]
    pub fn references(&self) -> &[ReferenceDef] {
        &self.references
    }

    /// Exact semantic model fingerprint.
    #[must_use]
    pub const fn fingerprint(&self) -> Fingerprint {
        self.fingerprint
    }
}

impl PartialEq for ModelDef {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
            && self.name == other.name
            && self.fields == other.fields
            && self.identity == other.identity
            && self.unique == other.unique
            && self.relations == other.relations
            && self.references == other.references
    }
}

impl Eq for ModelDef {}

fn fingerprint_model(parts: &ModelParts) -> Fingerprint {
    let mut hasher = CanonicalHasher::new(b"model/v1");
    hasher.str(parts.key.as_str());
    hasher.str(&parts.name);
    hasher.u64(parts.fields.len() as u64);

    for field in &parts.fields {
        hasher.str(field.key().as_str());
        hasher.str(field.name());
        hash_type_def(&mut hasher, field.ty());
        hasher.u8(match field.presence() {
            Presence::Required => 0,
            Presence::Optional => 1,
        });
    }

    match parts.identity.as_ref() {
        None => hasher.u8(0),
        Some(identity) => {
            hasher.u8(1);
            hash_field_keys(&mut hasher, identity.fields());
        }
    }

    hasher.u64(parts.unique.len() as u64);
    for constraint in &parts.unique {
        hash_field_keys(&mut hasher, constraint.fields());
    }

    hasher.u64(parts.relations.len() as u64);
    for relation in &parts.relations {
        hasher.str(relation.key().as_str());
        hasher.str(relation.name());
        hasher.str(relation.target_model().as_str());
        hasher.u8(match relation.cardinality() {
            super::Cardinality::One => 0,
            super::Cardinality::OptionalOne => 1,
            super::Cardinality::Many => 2,
        });
        hasher.u64(relation.fields().len() as u64);
        for pair in relation.fields() {
            hasher.str(pair.source().as_str());
            hasher.str(pair.target().as_str());
        }
    }

    hasher.u64(parts.references.len() as u64);
    for reference in &parts.references {
        hash_field_keys(&mut hasher, reference.source_fields());
        hasher.str(reference.target_model().as_str());
        hash_field_keys(&mut hasher, reference.target_fields());
    }

    hasher.finish()
}

fn hash_field_keys(hasher: &mut CanonicalHasher, fields: &[FieldKey]) {
    hasher.u64(fields.len() as u64);
    for field in fields {
        hasher.str(field.as_str());
    }
}
