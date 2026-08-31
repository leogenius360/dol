//! Physical model mappings owned by adapters.

/// Marker trait for backend-specific physical model mappings.
pub trait PhysicalMapping: Send + Sync + 'static {}
