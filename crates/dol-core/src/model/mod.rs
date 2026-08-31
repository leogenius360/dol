//! Logical models, fields, relations, constraints, and model environments.

mod builder;
mod constraint;
mod def;
mod field;
mod identity;
mod relation;
mod set;
mod traits;

pub use crate::types::Presence;
pub use builder::ModelBuilder;
pub use constraint::{IdentityDef, ReferenceDef, UniqueDef};
pub use def::ModelDef;
pub(crate) use def::ModelParts;
pub use field::{Field, FieldDef, FieldSlot};
pub use identity::{FieldKey, ModelKey, RelationKey};
pub use relation::{Cardinality, RelationDef, RelationField};
pub use set::ModelSet;
pub use traits::{Model, ModelOperations, RecordMut, RecordView};
