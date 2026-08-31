//! MongoDB collection and BSON-field mappings owned by the adapter.

use std::collections::{BTreeMap, BTreeSet};

use dol_core::diagnostic::{Diagnostic, Result};
use dol_core::fingerprint::Fingerprint;
use dol_core::model::{ModelDef, ModelKey};

/// Physical BSON field used for one stable DOL field key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldMapping {
    name: String,
}

impl FieldMapping {
    /// Creates a top-level BSON field mapping.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }

    /// Physical BSON field name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// Physical MongoDB collection mapping for one semantic model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionMapping {
    database: String,
    collection: String,
    fields: BTreeMap<String, FieldMapping>,
}

impl dol_engine::mapping::PhysicalMapping for CollectionMapping {}

impl CollectionMapping {
    /// Creates an empty collection mapping.
    #[must_use]
    pub fn new(database: impl Into<String>, collection: impl Into<String>) -> Self {
        Self {
            database: database.into(),
            collection: collection.into(),
            fields: BTreeMap::new(),
        }
    }

    /// Adds or replaces one field mapping by stable field key.
    #[must_use]
    pub fn field(mut self, key: impl Into<String>, mapping: FieldMapping) -> Self {
        self.fields.insert(key.into(), mapping);
        self
    }

    /// Database name.
    #[must_use]
    pub fn database(&self) -> &str {
        &self.database
    }

    /// Collection name.
    #[must_use]
    pub fn collection(&self) -> &str {
        &self.collection
    }

    /// Looks up a physical field by stable semantic key.
    #[must_use]
    pub fn field_mapping(&self, key: &str) -> Option<&FieldMapping> {
        self.fields.get(key)
    }

    fn validate(&self, model: &ModelDef) -> Result<()> {
        validate_database_name(&self.database)?;
        validate_collection_name(&self.collection)?;
        if self.fields.len() != model.fields().len() {
            return Err(Diagnostic::error(
                "MONGODB-MAP-001",
                "MongoDB mapping must cover every model field exactly once",
            ));
        }
        let mut physical = BTreeSet::new();
        for field in model.fields() {
            let mapping = self.fields.get(field.key().as_str()).ok_or_else(|| {
                Diagnostic::error(
                    "MONGODB-MAP-002",
                    format!("field `{}` has no BSON mapping", field.key().as_str()),
                )
            })?;
            validate_field_name(mapping.name())?;
            if !physical.insert(mapping.name().to_owned()) {
                return Err(Diagnostic::error(
                    "MONGODB-MAP-003",
                    "one BSON field is mapped to more than one DOL field",
                ));
            }
        }
        if let Some(key) = self.fields.keys().find(|key| model.field(key).is_none()) {
            return Err(Diagnostic::error(
                "MONGODB-MAP-004",
                format!("MongoDB mapping contains unknown field key `{key}`"),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct CatalogEntry {
    fingerprint: Fingerprint,
    mapping: CollectionMapping,
}

/// Adapter-owned semantic-model to collection registry.
#[derive(Debug, Clone, Default)]
pub struct MongodbCatalog {
    entries: BTreeMap<ModelKey, CatalogEntry>,
}

impl MongodbCatalog {
    /// Creates an empty catalog.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers one exact model and physical collection mapping.
    pub fn register(&mut self, model: &ModelDef, mapping: CollectionMapping) -> Result<()> {
        mapping.validate(model)?;
        if let Some(existing) = self.entries.get(model.key())
            && existing.fingerprint != model.fingerprint()
        {
            return Err(Diagnostic::error(
                "MONGODB-MAP-005",
                "MongoDB catalog already contains a different exact model definition for this lineage",
            ));
        }
        self.entries.insert(
            model.key().clone(),
            CatalogEntry {
                fingerprint: model.fingerprint(),
                mapping,
            },
        );
        Ok(())
    }

    /// Resolves and revalidates a collection mapping.
    pub fn collection(&self, model: &ModelDef) -> Result<&CollectionMapping> {
        let entry = self.entries.get(model.key()).ok_or_else(|| {
            Diagnostic::error(
                "MONGODB-MAP-006",
                format!("model `{}` is not registered", model.key().as_str()),
            )
        })?;
        if entry.fingerprint != model.fingerprint() {
            return Err(Diagnostic::error(
                "MONGODB-MAP-005",
                "registered MongoDB mapping belongs to a different exact model definition",
            ));
        }
        entry.mapping.validate(model)?;
        Ok(&entry.mapping)
    }
}

fn validate_database_name(value: &str) -> Result<()> {
    if value.is_empty() || value.len() > 63 || value.contains(['\0', '/', '\\', '.', ' ', '"', '$'])
    {
        return Err(Diagnostic::error(
            "MONGODB-MAP-007",
            "MongoDB database name is empty or contains unsupported characters",
        ));
    }
    Ok(())
}

fn validate_collection_name(value: &str) -> Result<()> {
    if value.is_empty() || value.contains('\0') || value.starts_with("system.") {
        return Err(Diagnostic::error(
            "MONGODB-MAP-008",
            "MongoDB collection name is empty, reserved, or contains NUL",
        ));
    }
    Ok(())
}

fn validate_field_name(value: &str) -> Result<()> {
    if value.is_empty()
        || value.contains(['\0', '.'])
        || value.starts_with('$')
        || value.starts_with("__dol_")
    {
        return Err(Diagnostic::error(
            "MONGODB-MAP-009",
            "BSON field names must be top-level, non-reserved, and contain neither NUL nor `.`",
        ));
    }
    Ok(())
}
