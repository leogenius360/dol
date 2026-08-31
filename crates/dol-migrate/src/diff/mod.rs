//! Deterministic catalog diffing with explicit rename intent.

use std::collections::{BTreeMap, BTreeSet};

use dol_core::diagnostic::{Diagnostic, Result};

use crate::catalog::{CatalogEntity, CatalogField, CatalogIndex, CatalogRevision, CatalogSnapshot};

/// Limits for one diff operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiffLimits {
    /// Maximum emitted operations.
    pub max_operations: usize,
    /// Maximum caller-declared renames.
    pub max_rename_intents: usize,
}

impl Default for DiffLimits {
    fn default() -> Self {
        Self {
            max_operations: 16_384,
            max_rename_intents: 4_096,
        }
    }
}

impl DiffLimits {
    fn validate(self) -> Result<()> {
        if self.max_operations == 0 || self.max_rename_intents == 0 {
            return Err(Diagnostic::error(
                "MIGRATE-DIFF-001",
                "diff limits must be greater than zero",
            ));
        }
        Ok(())
    }
}

/// An exact, caller-declared rename. Similarity never creates one implicitly.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RenameIntent {
    /// Rename an entity while preserving its stable key.
    Entity {
        /// Stable entity key.
        key: String,
        /// Required current name.
        from: String,
        /// Required target name.
        to: String,
    },
    /// Rename a field while preserving its stable key.
    Field {
        /// Stable entity key.
        entity_key: String,
        /// Stable field key.
        key: String,
        /// Required current name.
        from: String,
        /// Required target name.
        to: String,
    },
    /// Rename an index while preserving its stable key.
    Index {
        /// Stable entity key.
        entity_key: String,
        /// Stable index key.
        key: String,
        /// Required current name.
        from: String,
        /// Required target name.
        to: String,
    },
}

impl RenameIntent {
    /// Declares an entity rename.
    #[must_use]
    pub fn entity(key: impl Into<String>, from: impl Into<String>, to: impl Into<String>) -> Self {
        Self::Entity {
            key: key.into(),
            from: from.into(),
            to: to.into(),
        }
    }

    /// Declares a field rename.
    #[must_use]
    pub fn field(
        entity_key: impl Into<String>,
        key: impl Into<String>,
        from: impl Into<String>,
        to: impl Into<String>,
    ) -> Self {
        Self::Field {
            entity_key: entity_key.into(),
            key: key.into(),
            from: from.into(),
            to: to.into(),
        }
    }

    /// Declares an index rename.
    #[must_use]
    pub fn index(
        entity_key: impl Into<String>,
        key: impl Into<String>,
        from: impl Into<String>,
        to: impl Into<String>,
    ) -> Self {
        Self::Index {
            entity_key: entity_key.into(),
            key: key.into(),
            from: from.into(),
            to: to.into(),
        }
    }

    fn validate(&self) -> Result<()> {
        let values: &[&str] = match self {
            Self::Entity { key, from, to } => &[key, from, to],
            Self::Field {
                entity_key,
                key,
                from,
                to,
            }
            | Self::Index {
                entity_key,
                key,
                from,
                to,
            } => &[entity_key, key, from, to],
        };
        if values
            .iter()
            .any(|value| value.is_empty() || value.trim() != *value)
        {
            return Err(Diagnostic::error(
                "MIGRATE-DIFF-002",
                "rename keys and names must be non-empty and have no surrounding whitespace",
            ));
        }
        let (from, to) = match self {
            Self::Entity { from, to, .. }
            | Self::Field { from, to, .. }
            | Self::Index { from, to, .. } => (from, to),
        };
        if from == to {
            return Err(Diagnostic::error(
                "MIGRATE-DIFF-003",
                "rename source and target names must differ",
            ));
        }
        Ok(())
    }
}

/// Explicit semantic intent supplied to catalog diffing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MigrationIntent {
    renames: Vec<RenameIntent>,
}

impl MigrationIntent {
    /// Creates and validates a deterministic set of rename declarations.
    pub fn try_new(mut renames: Vec<RenameIntent>) -> Result<Self> {
        for rename in &renames {
            rename.validate()?;
        }
        renames.sort();
        if renames.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(Diagnostic::error(
                "MIGRATE-DIFF-004",
                "duplicate rename intent",
            ));
        }
        Ok(Self { renames })
    }

    /// Declared renames in canonical order.
    #[must_use]
    pub fn renames(&self) -> &[RenameIntent] {
        &self.renames
    }
}

/// One typed, backend-neutral catalog transition.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum MigrationOperation {
    /// Create a complete entity.
    CreateEntity { entity: CatalogEntity },
    /// Drop a complete entity.
    DropEntity { entity: CatalogEntity },
    /// Rename an entity.
    RenameEntity {
        entity_key: String,
        from: String,
        to: String,
    },
    /// Add a field to an existing entity.
    AddField {
        entity_key: String,
        field: CatalogField,
    },
    /// Drop a field from an existing entity.
    DropField {
        entity_key: String,
        field: CatalogField,
    },
    /// Rename a field.
    RenameField {
        entity_key: String,
        field_key: String,
        from: String,
        to: String,
    },
    /// Change a field's physical definition, excluding its name.
    AlterField {
        entity_key: String,
        before: CatalogField,
        after: CatalogField,
    },
    /// Create an index.
    CreateIndex {
        entity_key: String,
        index: CatalogIndex,
    },
    /// Drop an index.
    DropIndex {
        entity_key: String,
        index: CatalogIndex,
    },
    /// Rename an otherwise unchanged index.
    RenameIndex {
        entity_key: String,
        index_key: String,
        from: String,
        to: String,
    },
}

impl MigrationOperation {
    /// Concise, non-authoritative description for logs and review UIs.
    #[must_use]
    pub fn summary(&self) -> String {
        match self {
            Self::CreateEntity { entity } => format!("create entity `{}`", entity.name()),
            Self::DropEntity { entity } => format!("drop entity `{}`", entity.name()),
            Self::RenameEntity { from, to, .. } => {
                format!("rename entity `{from}` to `{to}`")
            }
            Self::AddField { entity_key, field } => {
                format!("add field `{}` to `{entity_key}`", field.name())
            }
            Self::DropField { entity_key, field } => {
                format!("drop field `{}` from `{entity_key}`", field.name())
            }
            Self::RenameField {
                entity_key,
                from,
                to,
                ..
            } => format!("rename field `{entity_key}.{from}` to `{to}`"),
            Self::AlterField {
                entity_key, after, ..
            } => format!("alter field `{entity_key}.{}`", after.name()),
            Self::CreateIndex { entity_key, index } => {
                format!("create index `{}` on `{entity_key}`", index.name())
            }
            Self::DropIndex { entity_key, index } => {
                format!("drop index `{}` from `{entity_key}`", index.name())
            }
            Self::RenameIndex {
                entity_key,
                from,
                to,
                ..
            } => format!("rename index `{entity_key}.{from}` to `{to}`"),
        }
    }
}

/// A deterministic transition between two exact catalog observations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogDiff {
    source: CatalogSnapshot,
    target: CatalogSnapshot,
    operations: Vec<MigrationOperation>,
}

impl CatalogDiff {
    /// Source revision precondition.
    #[must_use]
    pub const fn source_revision(&self) -> &CatalogRevision {
        self.source.revision()
    }

    /// Expected converged revision.
    #[must_use]
    pub const fn target_revision(&self) -> &CatalogRevision {
        self.target.revision()
    }

    /// Exact source snapshot.
    #[must_use]
    pub const fn source(&self) -> &CatalogSnapshot {
        &self.source
    }

    /// Exact target snapshot.
    #[must_use]
    pub const fn target(&self) -> &CatalogSnapshot {
        &self.target
    }

    /// Ordered operations.
    #[must_use]
    pub fn operations(&self) -> &[MigrationOperation] {
        &self.operations
    }

    /// Whether no catalog operation is required.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.operations.is_empty()
    }
}

/// Computes a bounded, deterministic catalog diff.
pub fn diff_catalog(
    source: &CatalogSnapshot,
    target: &CatalogSnapshot,
    intent: &MigrationIntent,
    limits: DiffLimits,
) -> Result<CatalogDiff> {
    limits.validate()?;
    if source.scope() != target.scope() {
        return Err(Diagnostic::error(
            "MIGRATE-DIFF-005",
            "source and target catalog scopes differ",
        ));
    }
    if intent.renames.len() > limits.max_rename_intents {
        return Err(Diagnostic::error(
            "MIGRATE-DIFF-006",
            format!(
                "rename intent count {} exceeds limit {}",
                intent.renames.len(),
                limits.max_rename_intents
            ),
        ));
    }

    let declared = intent.renames.iter().cloned().collect::<BTreeSet<_>>();
    let mut used = BTreeSet::new();
    let source_entities = source
        .entities()
        .iter()
        .map(|entity| (entity.key(), entity))
        .collect::<BTreeMap<_, _>>();
    let target_entities = target
        .entities()
        .iter()
        .map(|entity| (entity.key(), entity))
        .collect::<BTreeMap<_, _>>();

    let mut drop_indexes = Vec::new();
    let mut drop_fields = Vec::new();
    let mut drop_entities = Vec::new();
    let mut rename_entities = Vec::new();
    let mut create_entities = Vec::new();
    let mut rename_fields = Vec::new();
    let mut add_fields = Vec::new();
    let mut alter_fields = Vec::new();
    let mut rename_indexes = Vec::new();
    let mut create_indexes = Vec::new();

    for (key, entity) in &source_entities {
        if !target_entities.contains_key(key) {
            push_checked(
                &mut drop_entities,
                MigrationOperation::DropEntity {
                    entity: (*entity).clone(),
                },
                limits,
            )?;
        }
    }
    for (key, entity) in &target_entities {
        if !source_entities.contains_key(key) {
            push_checked(
                &mut create_entities,
                MigrationOperation::CreateEntity {
                    entity: (*entity).clone(),
                },
                limits,
            )?;
        }
    }

    for (key, before) in &source_entities {
        let Some(after) = target_entities.get(key) else {
            continue;
        };
        if before.name() != after.name() {
            let rename = RenameIntent::entity(*key, before.name(), after.name());
            require_declared(&rename, &declared, &mut used)?;
            push_checked(
                &mut rename_entities,
                MigrationOperation::RenameEntity {
                    entity_key: (*key).to_owned(),
                    from: before.name().to_owned(),
                    to: after.name().to_owned(),
                },
                limits,
            )?;
        }
        diff_entity(
            before,
            after,
            &mut DiffContext {
                declared: &declared,
                used: &mut used,
                limits,
            },
            EntityOperationBuckets {
                drop_indexes: &mut drop_indexes,
                drop_fields: &mut drop_fields,
                rename_fields: &mut rename_fields,
                add_fields: &mut add_fields,
                alter_fields: &mut alter_fields,
                rename_indexes: &mut rename_indexes,
                create_indexes: &mut create_indexes,
            },
        )?;
    }

    if used != declared {
        let unused = declared.difference(&used).next().expect("sets differ");
        return Err(Diagnostic::error(
            "MIGRATE-DIFF-007",
            format!("rename intent does not match an observed name change: {unused:?}"),
        ));
    }

    let mut operations = Vec::new();
    for bucket in [
        drop_indexes,
        drop_fields,
        drop_entities,
        rename_entities,
        create_entities,
        rename_fields,
        add_fields,
        alter_fields,
        rename_indexes,
        create_indexes,
    ] {
        append_checked(&mut operations, bucket, limits)?;
    }
    Ok(CatalogDiff {
        source: source.clone(),
        target: target.clone(),
        operations,
    })
}

struct EntityOperationBuckets<'a> {
    drop_indexes: &'a mut Vec<MigrationOperation>,
    drop_fields: &'a mut Vec<MigrationOperation>,
    rename_fields: &'a mut Vec<MigrationOperation>,
    add_fields: &'a mut Vec<MigrationOperation>,
    alter_fields: &'a mut Vec<MigrationOperation>,
    rename_indexes: &'a mut Vec<MigrationOperation>,
    create_indexes: &'a mut Vec<MigrationOperation>,
}

struct DiffContext<'a> {
    declared: &'a BTreeSet<RenameIntent>,
    used: &'a mut BTreeSet<RenameIntent>,
    limits: DiffLimits,
}

fn diff_entity(
    before: &CatalogEntity,
    after: &CatalogEntity,
    context: &mut DiffContext<'_>,
    buckets: EntityOperationBuckets<'_>,
) -> Result<()> {
    let before_fields = before
        .fields()
        .iter()
        .map(|field| (field.key(), field))
        .collect::<BTreeMap<_, _>>();
    let after_fields = after
        .fields()
        .iter()
        .map(|field| (field.key(), field))
        .collect::<BTreeMap<_, _>>();
    for (key, field) in &before_fields {
        if !after_fields.contains_key(key) {
            push_checked(
                buckets.drop_fields,
                MigrationOperation::DropField {
                    entity_key: before.key().to_owned(),
                    field: (*field).clone(),
                },
                context.limits,
            )?;
        }
    }
    for (key, field) in &after_fields {
        if !before_fields.contains_key(key) {
            push_checked(
                buckets.add_fields,
                MigrationOperation::AddField {
                    entity_key: after.key().to_owned(),
                    field: (*field).clone(),
                },
                context.limits,
            )?;
        }
    }
    for (key, old) in &before_fields {
        let Some(new) = after_fields.get(key) else {
            continue;
        };
        if old.name() != new.name() {
            let rename = RenameIntent::field(before.key(), *key, old.name(), new.name());
            require_declared(&rename, context.declared, context.used)?;
            push_checked(
                buckets.rename_fields,
                MigrationOperation::RenameField {
                    entity_key: before.key().to_owned(),
                    field_key: (*key).to_owned(),
                    from: old.name().to_owned(),
                    to: new.name().to_owned(),
                },
                context.limits,
            )?;
        }
        if old.data_type() != new.data_type()
            || old.required() != new.required()
            || old.nullable() != new.nullable()
        {
            push_checked(
                buckets.alter_fields,
                MigrationOperation::AlterField {
                    entity_key: before.key().to_owned(),
                    before: (*old).clone(),
                    after: (*new).clone(),
                },
                context.limits,
            )?;
        }
    }

    diff_indexes(before, after, context, buckets)
}

fn diff_indexes(
    before: &CatalogEntity,
    after: &CatalogEntity,
    context: &mut DiffContext<'_>,
    buckets: EntityOperationBuckets<'_>,
) -> Result<()> {
    let old_indexes = before
        .indexes()
        .iter()
        .map(|index| (index.key(), index))
        .collect::<BTreeMap<_, _>>();
    let new_indexes = after
        .indexes()
        .iter()
        .map(|index| (index.key(), index))
        .collect::<BTreeMap<_, _>>();
    for (key, old) in &old_indexes {
        let new = new_indexes.get(key);
        let definition_changed =
            new.is_some_and(|new| old.fields() != new.fields() || old.unique() != new.unique());
        if new.is_none() || definition_changed {
            push_checked(
                buckets.drop_indexes,
                MigrationOperation::DropIndex {
                    entity_key: before.key().to_owned(),
                    index: (*old).clone(),
                },
                context.limits,
            )?;
        }
    }
    for (key, new) in &new_indexes {
        let old = old_indexes.get(key);
        let definition_changed =
            old.is_some_and(|old| old.fields() != new.fields() || old.unique() != new.unique());
        if old.is_none() || definition_changed {
            push_checked(
                buckets.create_indexes,
                MigrationOperation::CreateIndex {
                    entity_key: after.key().to_owned(),
                    index: (*new).clone(),
                },
                context.limits,
            )?;
        }
        if let Some(old) = old
            && old.name() != new.name()
        {
            let rename = RenameIntent::index(before.key(), *key, old.name(), new.name());
            require_declared(&rename, context.declared, context.used)?;
            if !definition_changed {
                push_checked(
                    buckets.rename_indexes,
                    MigrationOperation::RenameIndex {
                        entity_key: before.key().to_owned(),
                        index_key: (*key).to_owned(),
                        from: old.name().to_owned(),
                        to: new.name().to_owned(),
                    },
                    context.limits,
                )?;
            }
        }
    }
    Ok(())
}

fn require_declared(
    rename: &RenameIntent,
    declared: &BTreeSet<RenameIntent>,
    used: &mut BTreeSet<RenameIntent>,
) -> Result<()> {
    if !declared.contains(rename) {
        return Err(Diagnostic::error(
            "MIGRATE-DIFF-008",
            format!("name change requires exact rename intent: {rename:?}"),
        ));
    }
    used.insert(rename.clone());
    Ok(())
}

fn push_checked(
    operations: &mut Vec<MigrationOperation>,
    operation: MigrationOperation,
    limits: DiffLimits,
) -> Result<()> {
    if operations.len() >= limits.max_operations {
        return Err(Diagnostic::error(
            "MIGRATE-DIFF-009",
            format!(
                "migration operation count exceeds limit {}",
                limits.max_operations
            ),
        ));
    }
    operations.push(operation);
    Ok(())
}

fn append_checked(
    operations: &mut Vec<MigrationOperation>,
    mut bucket: Vec<MigrationOperation>,
    limits: DiffLimits,
) -> Result<()> {
    let total = operations.len().checked_add(bucket.len()).ok_or_else(|| {
        Diagnostic::error("MIGRATE-DIFF-009", "migration operation count overflowed")
    })?;
    if total > limits.max_operations {
        return Err(Diagnostic::error(
            "MIGRATE-DIFF-009",
            format!(
                "migration operation count exceeds limit {}",
                limits.max_operations
            ),
        ));
    }
    operations.append(&mut bucket);
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::catalog::{CatalogRevision, CatalogScope};

    use super::*;

    fn field(key: &str, name: &str, ty: &str) -> CatalogField {
        CatalogField::try_new(key, name, ty, true, false).unwrap()
    }

    fn snapshot(revision: &str, entity_name: &str, field_name: &str) -> CatalogSnapshot {
        CatalogSnapshot::try_new(
            CatalogScope::try_new("fake", "test").unwrap(),
            CatalogRevision::try_new(revision).unwrap(),
            vec![
                CatalogEntity::try_new(
                    "user",
                    entity_name,
                    vec![field("id", "id", "u64"), field("email", field_name, "text")],
                    Vec::new(),
                )
                .unwrap(),
            ],
        )
        .unwrap()
    }

    #[test]
    fn name_changes_require_exact_explicit_intent() {
        let source = snapshot("one", "users", "email");
        let target = snapshot("two", "people", "email_address");
        let error = diff_catalog(
            &source,
            &target,
            &MigrationIntent::default(),
            DiffLimits::default(),
        )
        .unwrap_err();
        assert_eq!(error.code(), "MIGRATE-DIFF-008");

        let intent = MigrationIntent::try_new(vec![
            RenameIntent::entity("user", "users", "people"),
            RenameIntent::field("user", "email", "email", "email_address"),
        ])
        .unwrap();
        let diff = diff_catalog(&source, &target, &intent, DiffLimits::default()).unwrap();
        assert_eq!(diff.operations().len(), 2);
        assert!(matches!(
            diff.operations()[0],
            MigrationOperation::RenameEntity { .. }
        ));
        assert!(matches!(
            diff.operations()[1],
            MigrationOperation::RenameField { .. }
        ));
    }

    #[test]
    fn unused_rename_intent_is_rejected() {
        let source = snapshot("one", "users", "email");
        let target = snapshot("two", "users", "email");
        let intent =
            MigrationIntent::try_new(vec![RenameIntent::entity("user", "users", "people")])
                .unwrap();
        let error = diff_catalog(&source, &target, &intent, DiffLimits::default()).unwrap_err();
        assert_eq!(error.code(), "MIGRATE-DIFF-007");
    }
}
