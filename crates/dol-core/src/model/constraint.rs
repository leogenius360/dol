use super::{FieldKey, ModelKey};

/// Canonical model identity over one or more fields.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IdentityDef {
    fields: Vec<FieldKey>,
}

impl IdentityDef {
    /// Identity fields in canonical order.
    #[must_use]
    pub fn fields(&self) -> &[FieldKey] {
        &self.fields
    }

    pub(crate) fn new(mut fields: Vec<FieldKey>) -> Self {
        fields.sort();
        Self { fields }
    }
}

/// Unique field tuple. Null/missing values do not participate in uniqueness.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UniqueDef {
    fields: Vec<FieldKey>,
}

impl UniqueDef {
    /// Constrained fields in canonical order.
    #[must_use]
    pub fn fields(&self) -> &[FieldKey] {
        &self.fields
    }

    pub(crate) fn new(mut fields: Vec<FieldKey>) -> Self {
        fields.sort();
        Self { fields }
    }
}

/// Referential integrity constraint independent of relation navigation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ReferenceDef {
    source_fields: Vec<FieldKey>,
    target_model: ModelKey,
    target_fields: Vec<FieldKey>,
}

impl ReferenceDef {
    /// Local reference fields.
    #[must_use]
    pub fn source_fields(&self) -> &[FieldKey] {
        &self.source_fields
    }

    /// Target model.
    #[must_use]
    pub const fn target_model(&self) -> &ModelKey {
        &self.target_model
    }

    /// Target fields.
    #[must_use]
    pub fn target_fields(&self) -> &[FieldKey] {
        &self.target_fields
    }

    pub(crate) fn new(
        mut source_fields: Vec<FieldKey>,
        target_model: ModelKey,
        mut target_fields: Vec<FieldKey>,
    ) -> Self {
        if source_fields.len() == target_fields.len() {
            let mut pairs: Vec<_> = source_fields.into_iter().zip(target_fields).collect();
            pairs.sort_by(|left, right| left.0.cmp(&right.0));
            (source_fields, target_fields) = pairs.into_iter().unzip();
        }

        Self {
            source_fields,
            target_model,
            target_fields,
        }
    }
}
