use core::fmt;
use core::hash::{Hash, Hasher};

use crate::binding::{Binding, NativeBinding, NullableBinding, SemanticBinding};
use crate::types::{DataType, TypeDef};
use crate::value::DataValue;

use super::{FieldKey, Presence};

/// Dense model-local field slot used only for execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FieldSlot(u32);

impl FieldSlot {
    pub(crate) fn new(value: u32) -> Self {
        Self(value)
    }

    /// Zero-based slot index.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// Canonical logical field definition.
#[derive(Debug, Clone)]
pub struct FieldDef {
    key: FieldKey,
    name: String,
    slot: FieldSlot,
    ty: TypeDef,
    presence: Presence,
}

impl FieldDef {
    pub(crate) fn new(
        key: FieldKey,
        name: String,
        slot: FieldSlot,
        ty: TypeDef,
        presence: Presence,
    ) -> Self {
        Self {
            key,
            name,
            slot,
            ty,
            presence,
        }
    }

    /// Stable field lineage key.
    #[must_use]
    pub const fn key(&self) -> &FieldKey {
        &self.key
    }

    /// Current logical field name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Dense execution slot.
    #[must_use]
    pub const fn slot(&self) -> FieldSlot {
        self.slot
    }

    /// Semantic type definition.
    #[must_use]
    pub const fn ty(&self) -> &TypeDef {
        &self.ty
    }

    /// Presence requirement.
    #[must_use]
    pub const fn presence(&self) -> Presence {
        self.presence
    }
}

impl PartialEq for FieldDef {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
            && self.name == other.name
            && self.ty == other.ty
            && self.presence == other.presence
    }
}

impl Eq for FieldDef {}

impl Hash for FieldDef {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.key.hash(state);
        self.name.hash(state);
        self.ty.hash(state);
        self.presence.hash(state);
    }
}

/// Typed static field descriptor generated for Rust models.
pub struct Field<T> {
    model: &'static str,
    key: &'static str,
    name: &'static str,
    binding: Binding<T>,
    nullable_binding: Binding<Option<T>>,
}

impl<T> Field<T> {
    /// Creates a field with an explicit semantic binding.
    #[must_use]
    pub const fn with_binding<B>(model: &'static str, key: &'static str, name: &'static str) -> Self
    where
        B: SemanticBinding<T>,
    {
        Self {
            model,
            key,
            name,
            binding: Binding::of::<B>(),
            nullable_binding: Binding::of::<NullableBinding<B>>(),
        }
    }

    /// Stable semantic model key that owns this field.
    #[must_use]
    pub const fn model(self) -> &'static str {
        self.model
    }

    /// Stable semantic field key.
    #[must_use]
    pub const fn key(self) -> &'static str {
        self.key
    }

    /// Current logical field name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        self.name
    }

    /// Returns this field's exact DOL semantic type definition.
    #[must_use]
    pub fn type_def(self) -> TypeDef {
        self.binding.type_def()
    }

    pub(crate) const fn binding(self) -> Binding<T> {
        self.binding
    }

    pub(crate) const fn nullable_binding(self) -> Binding<Option<T>> {
        self.nullable_binding
    }
}

impl<T> Field<T>
where
    T: DataType + DataValue,
{
    /// Creates a field whose stable key equals its current name.
    #[must_use]
    pub const fn new(model: &'static str, name: &'static str) -> Self {
        Self::with_binding::<NativeBinding>(model, name, name)
    }

    /// Creates a field with an explicit model and stable semantic key.
    #[must_use]
    pub const fn with_key(model: &'static str, key: &'static str, name: &'static str) -> Self {
        Self::with_binding::<NativeBinding>(model, key, name)
    }
}

impl<T> Copy for Field<T> {}

impl<T> Clone for Field<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> fmt::Debug for Field<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Field")
            .field("model", &self.model)
            .field("key", &self.key)
            .field("name", &self.name)
            .field("type", &self.binding.type_def())
            .finish_non_exhaustive()
    }
}

impl<T> PartialEq for Field<T> {
    fn eq(&self, other: &Self) -> bool {
        self.model == other.model && self.key == other.key
    }
}

impl<T> Eq for Field<T> {}

impl<T> Hash for Field<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.model.hash(state);
        self.key.hash(state);
    }
}
