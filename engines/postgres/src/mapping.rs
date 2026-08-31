//! PostgreSQL physical table/column mappings with explicit optional-field presence.

use std::collections::{BTreeMap, BTreeSet};

use dol_core::diagnostic::{Diagnostic, Result};
use dol_core::fingerprint::Fingerprint;
use dol_core::model::{ModelDef, ModelKey, Presence};

/// Physical PostgreSQL column used for one DOL field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnMapping {
    value: String,
    presence: Option<String>,
}

impl ColumnMapping {
    /// Maps a required DOL field to one PostgreSQL value column.
    #[must_use]
    pub fn required(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            presence: None,
        }
    }

    /// Maps an optional DOL field to a value column plus explicit presence bit.
    #[must_use]
    pub fn optional(value: impl Into<String>, presence: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            presence: Some(presence.into()),
        }
    }

    /// Physical value column.
    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }

    /// Presence-bit column used to distinguish Missing from Null.
    #[must_use]
    pub fn presence(&self) -> Option<&str> {
        self.presence.as_deref()
    }
}

/// Physical PostgreSQL table mapping for one semantic model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableMapping {
    schema: String,
    table: String,
    fields: BTreeMap<String, ColumnMapping>,
}

impl dol_engine::mapping::PhysicalMapping for TableMapping {}

impl TableMapping {
    /// Creates an empty mapping for one qualified PostgreSQL table.
    #[must_use]
    pub fn new(schema: impl Into<String>, table: impl Into<String>) -> Self {
        Self {
            schema: schema.into(),
            table: table.into(),
            fields: BTreeMap::new(),
        }
    }

    /// Adds or replaces one field mapping by stable DOL field key.
    #[must_use]
    pub fn field(mut self, key: impl Into<String>, column: ColumnMapping) -> Self {
        self.fields.insert(key.into(), column);
        self
    }

    /// Physical schema name.
    #[must_use]
    pub fn schema(&self) -> &str {
        &self.schema
    }

    /// Physical table name.
    #[must_use]
    pub fn table(&self) -> &str {
        &self.table
    }

    /// Field mapping by stable semantic field key.
    #[must_use]
    pub fn field_mapping(&self, key: &str) -> Option<&ColumnMapping> {
        self.fields.get(key)
    }

    pub(crate) fn validate(&self, model: &ModelDef) -> Result<()> {
        validate_identifier(&self.schema, "schema")?;
        validate_identifier(&self.table, "table")?;

        if self.fields.len() != model.fields().len() {
            return Err(Diagnostic::error(
                "POSTGRES-MAP-001",
                "PostgreSQL mapping must cover every model field exactly once",
            ));
        }

        let mut physical = BTreeSet::new();
        for field in model.fields() {
            let mapping = self.fields.get(field.key().as_str()).ok_or_else(|| {
                Diagnostic::error(
                    "POSTGRES-MAP-002",
                    format!(
                        "field `{}` has no PostgreSQL column mapping",
                        field.key().as_str()
                    ),
                )
            })?;
            validate_identifier(mapping.value(), "column")?;
            if !physical.insert(mapping.value().to_owned()) {
                return Err(Diagnostic::error(
                    "POSTGRES-MAP-003",
                    "one PostgreSQL column is mapped to more than one DOL field/state",
                ));
            }

            match (field.presence(), mapping.presence()) {
                (Presence::Optional, None) => {
                    return Err(Diagnostic::error(
                        "POSTGRES-MAP-004",
                        format!(
                            "optional field `{}` requires a presence column so Missing and Null remain distinct",
                            field.key().as_str()
                        ),
                    ));
                }
                (Presence::Required, Some(_)) => {
                    return Err(Diagnostic::error(
                        "POSTGRES-MAP-005",
                        format!(
                            "required field `{}` must not declare an optional-field presence column",
                            field.key().as_str()
                        ),
                    ));
                }
                (_, Some(presence)) => {
                    validate_identifier(presence, "presence column")?;
                    if !physical.insert(presence.to_owned()) {
                        return Err(Diagnostic::error(
                            "POSTGRES-MAP-003",
                            "one PostgreSQL column is mapped to more than one DOL field/state",
                        ));
                    }
                }
                (_, None) => {}
            }
        }

        for key in self.fields.keys() {
            if model.field(key).is_none() {
                return Err(Diagnostic::error(
                    "POSTGRES-MAP-006",
                    format!("PostgreSQL mapping contains unknown field key `{key}`"),
                ));
            }
        }
        Ok(())
    }
}

/// One exact-model registration in the physical catalog.
#[derive(Debug, Clone)]
struct CatalogEntry {
    model_fingerprint: Fingerprint,
    table: TableMapping,
}

/// Adapter-owned registry from semantic model lineage to PostgreSQL tables.
#[derive(Debug, Clone, Default)]
pub struct PostgresCatalog {
    tables: BTreeMap<ModelKey, CatalogEntry>,
}

impl PostgresCatalog {
    /// Creates an empty physical mapping catalog.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers and validates one model/table mapping.
    pub fn register(&mut self, model: &ModelDef, mapping: TableMapping) -> Result<()> {
        mapping.validate(model)?;

        if let Some(existing) = self.tables.get(model.key())
            && existing.model_fingerprint != model.fingerprint()
        {
            return Err(Diagnostic::error(
                "POSTGRES-MAP-009",
                "PostgreSQL catalog already contains a different exact model definition for this model lineage",
            ));
        }

        self.tables.insert(
            model.key().clone(),
            CatalogEntry {
                model_fingerprint: model.fingerprint(),
                table: mapping,
            },
        );
        Ok(())
    }

    /// Returns the mapping for one semantic model.
    pub fn table(&self, model: &ModelDef) -> Result<&TableMapping> {
        let entry = self.tables.get(model.key()).ok_or_else(|| {
            Diagnostic::error(
                "POSTGRES-MAP-007",
                format!(
                    "model `{}` is not registered in the PostgreSQL catalog",
                    model.key().as_str()
                ),
            )
        })?;
        if entry.model_fingerprint != model.fingerprint() {
            return Err(Diagnostic::error(
                "POSTGRES-MAP-009",
                "registered PostgreSQL mapping belongs to a different exact model definition",
            ));
        }
        entry.table.validate(model)?;
        Ok(&entry.table)
    }
}

pub(crate) fn quote_identifier(value: &str) -> Result<String> {
    validate_identifier(value, "identifier")?;
    Ok(format!("\"{}\"", value.replace('"', "\"\"")))
}

fn validate_identifier(value: &str, kind: &str) -> Result<()> {
    if value.is_empty() || value.contains('\0') {
        return Err(Diagnostic::error(
            "POSTGRES-MAP-008",
            format!("PostgreSQL {kind} must be non-empty and contain no NUL byte"),
        ));
    }
    if value.len() > 63 {
        return Err(Diagnostic::error(
            "POSTGRES-MAP-010",
            format!("PostgreSQL {kind} exceeds the portable 63-byte identifier limit"),
        ));
    }
    Ok(())
}
