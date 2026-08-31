//! Runtime-defined models and dense dynamic records.

mod access;
mod row;

pub use access::RuntimeField;
pub use row::{DynRow, DynRowBuilder};
