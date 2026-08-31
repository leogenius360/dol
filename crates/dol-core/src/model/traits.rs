use crate::diagnostic::Result;
use crate::ops::{Delete, Insert, InsertMany, Unscoped, Update};
use crate::value::{Datum, DatumRef};

use super::{FieldSlot, ModelDef};

/// Static Rust type with canonical DOL model semantics.
pub trait Model: 'static {
    /// Returns this model's canonical immutable definition.
    fn model_def() -> Result<&'static ModelDef>;
}

/// One concrete record that exposes model metadata and slot-based borrowed data.
pub trait RecordView {
    /// Returns the model definition for this record.
    fn model(&self) -> Result<&ModelDef>;

    /// Borrows one field datum by dense slot.
    fn field(&self, slot: FieldSlot) -> Result<Option<DatumRef<'_>>>;
}

/// Concrete model record whose fields can be materialized from canonical DOL values.
///
/// This is an execution capability, not a new conceptual data abstraction. Static
/// `#[derive(Model)]` records implement it automatically; custom/foreign field
/// bindings may reject a write at runtime when they do not provide reverse
/// materialization.
pub trait RecordMut: RecordView {
    /// Replaces one field by dense model slot after semantic validation.
    fn set_field(&mut self, slot: FieldSlot, datum: Datum) -> Result<()>;
}

/// Adds first-class write constructors to a model.
pub trait ModelOperations: Model + Sized {
    /// Creates a single-value insert operation.
    #[must_use]
    fn insert(value: Self) -> Insert<Self> {
        Insert::new(value)
    }

    /// Creates a bulk insert operation.
    #[must_use]
    fn insert_many(values: impl IntoIterator<Item = Self>) -> InsertMany<Self> {
        InsertMany::new(values)
    }

    /// Creates an initially unscoped update operation.
    #[must_use]
    fn update() -> Update<Self, Unscoped> {
        Update::new()
    }

    /// Creates an initially unscoped delete operation.
    #[must_use]
    fn delete() -> Delete<Self, Unscoped> {
        Delete::new()
    }
}

impl<M: Model> ModelOperations for M {}
