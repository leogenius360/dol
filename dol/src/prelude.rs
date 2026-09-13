//! Common imports for DOL applications.

pub use dol_core::prelude::*;

#[cfg(feature = "derive")]
pub use dol_macros::{Model, Projection};

#[cfg(feature = "engine")]
pub use dol_engine::prelude::*;
