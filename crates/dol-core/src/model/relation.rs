use super::{FieldKey, ModelKey, RelationKey};

/// Logical relation cardinality.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Cardinality {
    /// Exactly one related record is expected.
    One,
    /// Zero or one related record is expected.
    OptionalOne,
    /// Zero or more related records may exist.
    Many,
}

/// One source-to-target field mapping in a relation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RelationField {
    source: FieldKey,
    target: FieldKey,
}

impl RelationField {
    /// Creates a field mapping.
    #[must_use]
    pub fn new(source: impl Into<String>, target: impl Into<String>) -> Self {
        Self {
            source: FieldKey::new(source),
            target: FieldKey::new(target),
        }
    }

    /// Source field key.
    #[must_use]
    pub const fn source(&self) -> &FieldKey {
        &self.source
    }

    /// Target field key.
    #[must_use]
    pub const fn target(&self) -> &FieldKey {
        &self.target
    }
}

/// Logical navigation/join relationship between models.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RelationDef {
    key: RelationKey,
    name: String,
    target_model: ModelKey,
    cardinality: Cardinality,
    fields: Vec<RelationField>,
}

impl RelationDef {
    /// Creates a logical relation definition.
    #[must_use]
    pub fn new<I, S, T>(
        key: impl Into<String>,
        name: impl Into<String>,
        target_model: impl Into<String>,
        cardinality: Cardinality,
        fields: I,
    ) -> Self
    where
        I: IntoIterator<Item = (S, T)>,
        S: Into<String>,
        T: Into<String>,
    {
        let mut fields: Vec<_> = fields
            .into_iter()
            .map(|(source, target)| RelationField::new(source, target))
            .collect();
        fields.sort_by(|left, right| {
            (left.source(), left.target()).cmp(&(right.source(), right.target()))
        });

        Self {
            key: RelationKey::new(key),
            name: name.into(),
            target_model: ModelKey::new(target_model),
            cardinality,
            fields,
        }
    }

    /// Stable relation key.
    #[must_use]
    pub const fn key(&self) -> &RelationKey {
        &self.key
    }

    /// Current logical relation name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Target model.
    #[must_use]
    pub const fn target_model(&self) -> &ModelKey {
        &self.target_model
    }

    /// Cardinality.
    #[must_use]
    pub const fn cardinality(&self) -> Cardinality {
        self.cardinality
    }

    /// Field mappings.
    #[must_use]
    pub fn fields(&self) -> &[RelationField] {
        &self.fields
    }
}
