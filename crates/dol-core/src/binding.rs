//! Rust-to-DOL semantic bindings, including foreign-type adapters.

use core::fmt;
use core::marker::PhantomData;

use crate::diagnostic::{Diagnostic, Result};
use crate::types::{DataType, TypeDef};
use crate::value::{DataValue, Datum, DatumRef};

/// Supplies portable DOL semantics for a Rust type `T`.
///
/// Application-owned types normally implement [`DataType`] and [`DataValue`]
/// directly. `SemanticBinding<T>` exists for foreign Rust types where the
/// orphan rules prevent those trait implementations. A local zero-sized
/// binding type can implement this trait for the foreign type instead.
pub trait SemanticBinding<T>: 'static {
    /// Exact semantic type definition represented by this binding.
    fn type_def() -> TypeDef;

    /// Borrows one Rust value through DOL's canonical datum representation.
    fn datum_ref(value: &T) -> DatumRef<'_>;

    /// Copies one Rust value into DOL's canonical owned datum representation.
    #[must_use]
    fn to_datum(value: &T) -> Datum {
        Self::datum_ref(value).into_owned_datum()
    }

    /// Reconstructs the Rust value from DOL's canonical representation.
    ///
    /// Foreign bindings that participate in local materialization or local
    /// updates should override this method. Read-only bindings may keep the
    /// explicit unsupported default.
    fn from_datum(_datum: Datum) -> Result<T> {
        Err(Diagnostic::error(
            "BINDING-DECODE-001",
            "this semantic binding does not provide canonical Rust materialization",
        ))
    }
}

/// Internal binding adapter that preserves `B` while lifting one value into
/// DOL nullable semantics.
pub(crate) struct NullableBinding<B>(PhantomData<fn() -> B>);

impl<T, B> SemanticBinding<Option<T>> for NullableBinding<B>
where
    B: SemanticBinding<T>,
{
    fn type_def() -> TypeDef {
        B::type_def().nullable()
    }

    fn datum_ref(value: &Option<T>) -> DatumRef<'_> {
        match value {
            Some(value) => B::datum_ref(value),
            None => DatumRef::Null,
        }
    }

    fn from_datum(datum: Datum) -> Result<Option<T>> {
        match datum {
            Datum::Null => Ok(None),
            Datum::Missing => Err(Diagnostic::error(
                "BINDING-DECODE-002",
                "missing cannot materialize as a nullable Rust value",
            )),
            value => B::from_datum(value).map(Some),
        }
    }
}

/// Default binding for Rust types that implement DOL's native semantic traits.
#[derive(Debug, Clone, Copy, Default)]
pub struct NativeBinding;

impl<T> SemanticBinding<T> for NativeBinding
where
    T: DataType + DataValue,
{
    fn type_def() -> TypeDef {
        T::type_def()
    }

    fn datum_ref(value: &T) -> DatumRef<'_> {
        value.datum_ref()
    }

    fn from_datum(datum: Datum) -> Result<T> {
        T::from_datum(datum)
    }
}

/// Copyable runtime witness for the semantic binding of Rust type `T`.
///
/// The witness stores only function pointers. It is safe to copy into field,
/// parameter, and expression descriptors and never depends on process-local
/// Rust type identity such as `std::any::TypeId` or `type_name`.
pub struct Binding<T> {
    type_def: fn() -> TypeDef,
    datum_ref: for<'a> fn(&'a T) -> DatumRef<'a>,
    _marker: PhantomData<fn() -> T>,
}

impl<T> Binding<T> {
    /// Creates a witness backed by semantic binding `B`.
    #[must_use]
    pub const fn of<B>() -> Self
    where
        B: SemanticBinding<T>,
    {
        Self {
            type_def: B::type_def,
            datum_ref: B::datum_ref,
            _marker: PhantomData,
        }
    }

    /// Returns the exact semantic type definition.
    #[must_use]
    pub fn type_def(self) -> TypeDef {
        (self.type_def)()
    }

    /// Borrows a Rust value through the canonical datum representation.
    #[must_use]
    pub fn datum_ref<'a>(self, value: &'a T) -> DatumRef<'a> {
        (self.datum_ref)(value)
    }

    /// Copies a Rust value into the canonical owned datum representation.
    #[must_use]
    pub fn to_datum(self, value: &T) -> Datum {
        self.datum_ref(value).into_owned_datum()
    }
}

impl<T> Copy for Binding<T> {}

impl<T> Clone for Binding<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> fmt::Debug for Binding<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Binding")
            .field("type", &self.type_def())
            .finish_non_exhaustive()
    }
}

impl<T> Binding<T>
where
    T: DataType + DataValue,
{
    /// Returns the default native DOL binding for `T`.
    #[must_use]
    pub const fn native() -> Self {
        Self::of::<NativeBinding>()
    }
}

/// Value paired explicitly with a semantic binding.
///
/// This is primarily useful for foreign Rust types used as expression
/// literals. Static model fields can instead declare the binding once through
/// `#[dol(with = ...)]`.
pub struct BoundValue<T, B> {
    value: T,
    _binding: PhantomData<fn() -> B>,
}

impl<T, B> BoundValue<T, B> {
    /// Wraps a value with binding `B`.
    #[must_use]
    pub const fn new(value: T) -> Self {
        Self {
            value,
            _binding: PhantomData,
        }
    }

    /// Returns the wrapped Rust value.
    #[must_use]
    pub const fn value(&self) -> &T {
        &self.value
    }

    /// Consumes the wrapper and returns the Rust value.
    #[must_use]
    pub fn into_inner(self) -> T {
        self.value
    }
}

impl<T: fmt::Debug, B> fmt::Debug for BoundValue<T, B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("BoundValue").field(&self.value).finish()
    }
}

/// Pairs a value with an explicit semantic binding.
#[must_use]
pub const fn bind_value<B, T>(value: T) -> BoundValue<T, B>
where
    B: SemanticBinding<T>,
{
    BoundValue::new(value)
}
