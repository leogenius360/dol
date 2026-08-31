//! Backend-neutral catalog snapshots and inspection contracts.

use std::collections::{BTreeMap, BTreeSet};

use dol_core::diagnostic::{Diagnostic, Result};

const MAX_CATALOG_TEXT_BYTES: usize = 1_024;

/// Resource bounds used while constructing or inspecting a catalog snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatalogLimits {
    /// Maximum entities in one snapshot.
    pub max_entities: usize,
    /// Maximum fields in one entity.
    pub max_fields_per_entity: usize,
    /// Maximum indexes in one entity.
    pub max_indexes_per_entity: usize,
    /// Maximum field references in one index.
    pub max_index_fields: usize,
}

impl Default for CatalogLimits {
    fn default() -> Self {
        Self {
            max_entities: 4_096,
            max_fields_per_entity: 4_096,
            max_indexes_per_entity: 1_024,
            max_index_fields: 64,
        }
    }
}

impl CatalogLimits {
    pub(crate) fn validate(self) -> Result<()> {
        if self.max_entities == 0
            || self.max_fields_per_entity == 0
            || self.max_indexes_per_entity == 0
            || self.max_index_fields == 0
        {
            return Err(Diagnostic::error(
                "MIGRATE-CATALOG-001",
                "catalog limits must all be greater than zero",
            ));
        }
        Ok(())
    }
}

/// Backend and namespace identifying the exact catalog being migrated.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CatalogScope {
    backend: String,
    namespace: String,
}

impl CatalogScope {
    /// Creates a validated catalog scope.
    pub fn try_new(backend: impl Into<String>, namespace: impl Into<String>) -> Result<Self> {
        let backend = backend.into();
        let namespace = namespace.into();
        validate_text("backend", &backend)?;
        validate_text("namespace", &namespace)?;
        Ok(Self { backend, namespace })
    }

    /// Backend family or engine kind.
    #[must_use]
    pub fn backend(&self) -> &str {
        &self.backend
    }

    /// Backend-local namespace, database, or schema.
    #[must_use]
    pub fn namespace(&self) -> &str {
        &self.namespace
    }
}

/// Opaque, backend-produced token for one exact observed catalog state.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CatalogRevision(String);

impl CatalogRevision {
    /// Creates a validated opaque revision token.
    pub fn try_new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        validate_text("catalog revision", &value)?;
        Ok(Self(value))
    }

    /// Returns the opaque token.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One catalog field, matched across revisions by its stable key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CatalogField {
    key: String,
    name: String,
    data_type: String,
    required: bool,
    nullable: bool,
}

impl CatalogField {
    /// Creates a validated catalog field.
    pub fn try_new(
        key: impl Into<String>,
        name: impl Into<String>,
        data_type: impl Into<String>,
        required: bool,
        nullable: bool,
    ) -> Result<Self> {
        let key = key.into();
        let name = name.into();
        let data_type = data_type.into();
        validate_text("field key", &key)?;
        validate_text("field name", &name)?;
        validate_text("field data type", &data_type)?;
        Ok(Self {
            key,
            name,
            data_type,
            required,
            nullable,
        })
    }

    /// Stable field lineage key.
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Current physical field name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Backend-normalized physical type descriptor.
    #[must_use]
    pub fn data_type(&self) -> &str {
        &self.data_type
    }

    /// Whether a value must be present.
    #[must_use]
    pub const fn required(&self) -> bool {
        self.required
    }

    /// Whether an explicit null is accepted.
    #[must_use]
    pub const fn nullable(&self) -> bool {
        self.nullable
    }
}

/// One backend index, matched across revisions by its stable key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CatalogIndex {
    key: String,
    name: String,
    fields: Vec<String>,
    unique: bool,
}

impl CatalogIndex {
    /// Creates a validated index. Field references are stable field keys.
    pub fn try_new(
        key: impl Into<String>,
        name: impl Into<String>,
        fields: impl IntoIterator<Item = impl Into<String>>,
        unique: bool,
    ) -> Result<Self> {
        let key = key.into();
        let name = name.into();
        let fields = fields.into_iter().map(Into::into).collect::<Vec<_>>();
        validate_text("index key", &key)?;
        validate_text("index name", &name)?;
        if fields.is_empty() {
            return Err(Diagnostic::error(
                "MIGRATE-CATALOG-002",
                "an index must reference at least one field",
            ));
        }
        let mut seen = BTreeSet::new();
        for field in &fields {
            validate_text("index field key", field)?;
            if !seen.insert(field) {
                return Err(Diagnostic::error(
                    "MIGRATE-CATALOG-003",
                    format!("index `{key}` repeats field key `{field}`"),
                ));
            }
        }
        Ok(Self {
            key,
            name,
            fields,
            unique,
        })
    }

    /// Stable index key.
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Current physical index name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Ordered stable field keys participating in the index.
    #[must_use]
    pub fn fields(&self) -> &[String] {
        &self.fields
    }

    /// Whether the index enforces uniqueness.
    #[must_use]
    pub const fn unique(&self) -> bool {
        self.unique
    }
}

/// A table, collection, or equivalent independently addressable catalog entity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogEntity {
    key: String,
    name: String,
    fields: Vec<CatalogField>,
    indexes: Vec<CatalogIndex>,
}

impl CatalogEntity {
    /// Creates, validates, and canonically orders a catalog entity.
    pub fn try_new(
        key: impl Into<String>,
        name: impl Into<String>,
        fields: Vec<CatalogField>,
        indexes: Vec<CatalogIndex>,
    ) -> Result<Self> {
        Self::try_new_with_limits(key, name, fields, indexes, CatalogLimits::default())
    }

    /// Creates an entity under explicit resource limits.
    pub fn try_new_with_limits(
        key: impl Into<String>,
        name: impl Into<String>,
        mut fields: Vec<CatalogField>,
        mut indexes: Vec<CatalogIndex>,
        limits: CatalogLimits,
    ) -> Result<Self> {
        limits.validate()?;
        let key = key.into();
        let name = name.into();
        validate_text("entity key", &key)?;
        validate_text("entity name", &name)?;
        if fields.len() > limits.max_fields_per_entity {
            return Err(limit_error(
                "fields",
                fields.len(),
                limits.max_fields_per_entity,
            ));
        }
        if indexes.len() > limits.max_indexes_per_entity {
            return Err(limit_error(
                "indexes",
                indexes.len(),
                limits.max_indexes_per_entity,
            ));
        }
        fields.sort_by(|left, right| left.key.cmp(&right.key));
        indexes.sort_by(|left, right| left.key.cmp(&right.key));
        validate_unique(&fields, CatalogField::key, CatalogField::name, "field")?;
        validate_unique(&indexes, CatalogIndex::key, CatalogIndex::name, "index")?;

        let field_keys = fields
            .iter()
            .map(CatalogField::key)
            .collect::<BTreeSet<_>>();
        for index in &indexes {
            if index.fields.len() > limits.max_index_fields {
                return Err(limit_error(
                    "index fields",
                    index.fields.len(),
                    limits.max_index_fields,
                ));
            }
            for field in &index.fields {
                if !field_keys.contains(field.as_str()) {
                    return Err(Diagnostic::error(
                        "MIGRATE-CATALOG-004",
                        format!(
                            "index `{}` references unknown field key `{field}` in entity `{key}`",
                            index.key
                        ),
                    ));
                }
            }
        }

        Ok(Self {
            key,
            name,
            fields,
            indexes,
        })
    }

    /// Stable entity lineage key.
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Current physical entity name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Canonically key-ordered fields.
    #[must_use]
    pub fn fields(&self) -> &[CatalogField] {
        &self.fields
    }

    /// Canonically key-ordered indexes.
    #[must_use]
    pub fn indexes(&self) -> &[CatalogIndex] {
        &self.indexes
    }

    /// Looks up a field by stable key.
    #[must_use]
    pub fn field(&self, key: &str) -> Option<&CatalogField> {
        self.fields
            .binary_search_by_key(&key, |field| field.key())
            .ok()
            .map(|index| &self.fields[index])
    }

    /// Looks up an index by stable key.
    #[must_use]
    pub fn index(&self, key: &str) -> Option<&CatalogIndex> {
        self.indexes
            .binary_search_by_key(&key, |index| index.key())
            .ok()
            .map(|position| &self.indexes[position])
    }
}

/// An immutable, validated observation of one complete catalog scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogSnapshot {
    scope: CatalogScope,
    revision: CatalogRevision,
    entities: Vec<CatalogEntity>,
}

impl CatalogSnapshot {
    /// Creates a snapshot using the default catalog limits.
    pub fn try_new(
        scope: CatalogScope,
        revision: CatalogRevision,
        entities: Vec<CatalogEntity>,
    ) -> Result<Self> {
        Self::try_new_with_limits(scope, revision, entities, CatalogLimits::default())
    }

    /// Creates a snapshot under explicit catalog limits.
    pub fn try_new_with_limits(
        scope: CatalogScope,
        revision: CatalogRevision,
        mut entities: Vec<CatalogEntity>,
        limits: CatalogLimits,
    ) -> Result<Self> {
        limits.validate()?;
        if entities.len() > limits.max_entities {
            return Err(limit_error("entities", entities.len(), limits.max_entities));
        }
        entities.sort_by(|left, right| left.key.cmp(&right.key));
        validate_unique(&entities, CatalogEntity::key, CatalogEntity::name, "entity")?;
        Ok(Self {
            scope,
            revision,
            entities,
        })
    }

    /// Catalog scope.
    #[must_use]
    pub const fn scope(&self) -> &CatalogScope {
        &self.scope
    }

    /// Exact observed revision.
    #[must_use]
    pub const fn revision(&self) -> &CatalogRevision {
        &self.revision
    }

    /// Canonically key-ordered entities.
    #[must_use]
    pub fn entities(&self) -> &[CatalogEntity] {
        &self.entities
    }

    /// Looks up an entity by stable key.
    #[must_use]
    pub fn entity(&self, key: &str) -> Option<&CatalogEntity> {
        self.entities
            .binary_search_by_key(&key, |entity| entity.key())
            .ok()
            .map(|index| &self.entities[index])
    }

    /// Returns whether two snapshots describe the same scope and definitions.
    /// Revision tokens are deliberately ignored.
    #[must_use]
    pub fn equivalent_catalog(&self, other: &Self) -> bool {
        self.scope == other.scope && self.entities == other.entities
    }
}

/// Reads a bounded, revisioned catalog snapshot from a backend.
pub trait CatalogInspector {
    /// Inspects the named scope. Implementations must reject observations that
    /// cannot be represented within `limits` rather than silently truncating.
    fn inspect(&mut self, scope: &CatalogScope, limits: CatalogLimits) -> Result<CatalogSnapshot>;
}

fn validate_text(label: &str, value: &str) -> Result<()> {
    if value.is_empty() || value.trim() != value {
        return Err(Diagnostic::error(
            "MIGRATE-CATALOG-005",
            format!("{label} must be non-empty and have no surrounding whitespace"),
        ));
    }
    if value.len() > MAX_CATALOG_TEXT_BYTES || value.chars().any(char::is_control) {
        return Err(Diagnostic::error(
            "MIGRATE-CATALOG-006",
            format!("{label} contains unsupported text or exceeds its byte limit"),
        ));
    }
    Ok(())
}

fn validate_unique<T>(
    values: &[T],
    key: impl Fn(&T) -> &str,
    name: impl Fn(&T) -> &str,
    label: &str,
) -> Result<()> {
    let mut keys = BTreeSet::new();
    let mut names = BTreeMap::new();
    for value in values {
        if !keys.insert(key(value)) {
            return Err(Diagnostic::error(
                "MIGRATE-CATALOG-007",
                format!("duplicate {label} key `{}`", key(value)),
            ));
        }
        if let Some(previous) = names.insert(name(value), key(value)) {
            return Err(Diagnostic::error(
                "MIGRATE-CATALOG-008",
                format!(
                    "duplicate {label} name `{}` for keys `{previous}` and `{}`",
                    name(value),
                    key(value)
                ),
            ));
        }
    }
    Ok(())
}

fn limit_error(label: &str, actual: usize, maximum: usize) -> Diagnostic {
    Diagnostic::error(
        "MIGRATE-CATALOG-009",
        format!("catalog {label} count {actual} exceeds limit {maximum}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_is_canonical_and_validates_index_references() {
        let id = CatalogField::try_new("id", "id", "u64", true, false).unwrap();
        let name = CatalogField::try_new("name", "name", "text", true, false).unwrap();
        let index = CatalogIndex::try_new("by_name", "by_name", ["name"], false).unwrap();
        let entity = CatalogEntity::try_new("user", "users", vec![name, id], vec![index]).unwrap();
        assert_eq!(entity.fields()[0].key(), "id");
        assert!(entity.field("name").is_some());

        let bad = CatalogIndex::try_new("bad", "bad", ["missing"], false).unwrap();
        let error = CatalogEntity::try_new("user", "users", entity.fields().to_vec(), vec![bad])
            .unwrap_err();
        assert_eq!(error.code(), "MIGRATE-CATALOG-004");
    }

    #[test]
    fn duplicate_physical_names_are_rejected() {
        let first = CatalogField::try_new("first", "same", "u64", true, false).unwrap();
        let second = CatalogField::try_new("second", "same", "u64", true, false).unwrap();
        let error =
            CatalogEntity::try_new("user", "users", vec![first, second], Vec::new()).unwrap_err();
        assert_eq!(error.code(), "MIGRATE-CATALOG-008");
    }
}
