//! Open semantic data-type system.

mod def;
mod traits;
mod validate;

pub use def::{
    Nullability, Presence, RecordField, ScalarRepr, TypeDef, TypeFingerprint, TypeKey,
    TypeParameter, TypeProperties, TypeShape,
};
pub use traits::{DataType, ScalarType};
pub(crate) use validate::type_node_count;
pub use validate::{validate_type, validate_type_universe};
