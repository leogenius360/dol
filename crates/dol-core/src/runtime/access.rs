use core::hash::{Hash, Hasher};

use crate::binding::{Binding, NativeBinding, NullableBinding, SemanticBinding};
use crate::diagnostic::{Diagnostic, Result};
use crate::model::{FieldKey, FieldSlot, ModelDef, ModelKey};
use crate::types::DataType;
use crate::value::DataValue;

/// Typed field handle resolved from a runtime model exactly once.
///
/// The handle retains semantic model/field identity for validation while the
/// dense slot supports execution without repeated name lookup. A binding
/// witness preserves the Rust-to-DOL semantic mapping, including foreign Rust
/// types adapted through [`SemanticBinding`].
pub struct RuntimeField<T> {
    model: ModelKey,
    key: FieldKey,
    slot: FieldSlot,
    binding: Binding<T>,
    nullable_binding: Binding<Option<T>>,
}

impl<T> Clone for RuntimeField<T> {
    fn clone(&self) -> Self {
        Self {
            model: self.model.clone(),
            key: self.key.clone(),
            slot: self.slot,
            binding: self.binding,
            nullable_binding: self.nullable_binding,
        }
    }
}

impl<T> core::fmt::Debug for RuntimeField<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RuntimeField")
            .field("model", &self.model)
            .field("key", &self.key)
            .field("slot", &self.slot)
            .field("type", &self.binding.type_def())
            .finish_non_exhaustive()
    }
}

impl<T> PartialEq for RuntimeField<T> {
    fn eq(&self, other: &Self) -> bool {
        self.model == other.model && self.key == other.key
    }
}

impl<T> Eq for RuntimeField<T> {}

impl<T> Hash for RuntimeField<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.model.hash(state);
        self.key.hash(state);
    }
}

impl<T> RuntimeField<T> {
    /// Model this field belongs to.
    #[must_use]
    pub const fn model(&self) -> &ModelKey {
        &self.model
    }

    /// Stable semantic field key.
    #[must_use]
    pub const fn key(&self) -> &FieldKey {
        &self.key
    }

    /// Dense execution slot.
    #[must_use]
    pub const fn slot(&self) -> FieldSlot {
        self.slot
    }

    /// Exact semantic type represented by this runtime field handle.
    #[must_use]
    pub fn type_def(&self) -> crate::types::TypeDef {
        self.binding.type_def()
    }

    pub(crate) const fn binding(&self) -> Binding<T> {
        self.binding
    }

    pub(crate) const fn nullable_binding(&self) -> Binding<Option<T>> {
        self.nullable_binding
    }
}

impl ModelDef {
    /// Resolves a stable field key and verifies its native Rust/DOL semantic type.
    pub fn runtime_field<T>(&self, key: &str) -> Result<RuntimeField<T>>
    where
        T: DataType + DataValue,
    {
        self.runtime_field_with::<T, NativeBinding>(key)
    }

    /// Resolves a stable field key through an explicit semantic binding.
    pub fn runtime_field_with<T, B>(&self, key: &str) -> Result<RuntimeField<T>>
    where
        B: SemanticBinding<T>,
    {
        let field = self.field(key).ok_or_else(|| {
            Diagnostic::error("RUNTIME-101", format!("unknown runtime field `{key}`"))
        })?;
        let binding = Binding::of::<B>();
        let expected = binding.type_def();
        if field.ty() != &expected {
            return Err(Diagnostic::error(
                "RUNTIME-102",
                format!(
                    "runtime field `{key}` has type `{}@{}` but `{}@{}` was requested",
                    field.ty().key().as_str(),
                    field.ty().version(),
                    expected.key().as_str(),
                    expected.version()
                ),
            ));
        }

        Ok(RuntimeField {
            model: self.key().clone(),
            key: field.key().clone(),
            slot: field.slot(),
            binding,
            nullable_binding: Binding::of::<NullableBinding<B>>(),
        })
    }

    /// Resolves a current logical field name and verifies its native Rust/DOL type.
    pub fn runtime_field_named<T>(&self, name: &str) -> Result<RuntimeField<T>>
    where
        T: DataType + DataValue,
    {
        self.runtime_field_named_with::<T, NativeBinding>(name)
    }

    /// Resolves a current logical field name through an explicit semantic binding.
    pub fn runtime_field_named_with<T, B>(&self, name: &str) -> Result<RuntimeField<T>>
    where
        B: SemanticBinding<T>,
    {
        let field = self.field_named(name).ok_or_else(|| {
            Diagnostic::error(
                "RUNTIME-101",
                format!("unknown runtime field name `{name}`"),
            )
        })?;
        self.runtime_field_with::<T, B>(field.key().as_str())
    }
}
