//! Untrusted-input decode limits.

/// Hard limits evaluated before or during wire lowering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodeLimits {
    /// Maximum complete frame bytes, including the fixed header.
    pub max_total_bytes: usize,
    /// Maximum decoded nodes.
    pub max_nodes: usize,
    /// Maximum structural depth.
    pub max_depth: usize,
    /// Maximum UTF-8 string bytes.
    pub max_string_bytes: usize,
    /// Maximum literal bytes.
    pub max_literal_bytes: usize,
    /// Maximum list items.
    pub max_list_items: usize,
}

impl Default for DecodeLimits {
    fn default() -> Self {
        Self {
            max_total_bytes: 8 * 1024 * 1024,
            max_nodes: 65_536,
            max_depth: 128,
            max_string_bytes: 1024 * 1024,
            max_literal_bytes: 4 * 1024 * 1024,
            max_list_items: 65_536,
        }
    }
}

impl DecodeLimits {
    /// Creates limits that reject every non-empty payload.
    #[must_use]
    pub const fn deny_all() -> Self {
        Self {
            max_total_bytes: 0,
            max_nodes: 0,
            max_depth: 0,
            max_string_bytes: 0,
            max_literal_bytes: 0,
            max_list_items: 0,
        }
    }
}
