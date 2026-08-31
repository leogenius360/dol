//! Conservative cache admission and lifetime policy.

use dol_core::diagnostic::{Diagnostic, Result};

/// Whether a compiled backend artifact can cross an execution-request boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ArtifactCacheability {
    /// The artifact contains secrets, handles, or other state that must not be cached.
    Never,
    /// The artifact embeds request values and may only be retained inside that request.
    RequestScoped,
    /// The artifact is independent of parameter values and may be keyed by plan/backend contract.
    PlanReusable,
}

/// Required lifetime of an admitted cache entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CacheLifetime {
    /// Entry must be destroyed with the execution request that created it.
    Request,
    /// Entry may be shared across requests under an exact semantic cache key.
    Shared,
}

/// Explicit bounded cache-admission policy.
///
/// This type deliberately does not implement a cache. It centralizes the safety
/// decision a cache implementation must make before retaining logical plans or
/// backend artifacts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CachePolicy {
    /// Whether validated logical plans may be cached by semantic fingerprint.
    pub cache_logical_plans: bool,
    /// Whether compiled artifacts may be retained when their declared lifetime is honored.
    pub cache_compiled_artifacts: bool,
    /// Maximum entries across one cache instance.
    pub max_entries: usize,
    /// Maximum total logical bytes across one cache instance.
    pub max_bytes: u64,
    /// Maximum logical bytes admitted for any single entry.
    pub max_entry_bytes: u64,
}

impl CachePolicy {
    /// Validates that every enabled cache has finite, non-zero bounds.
    pub fn validate(self) -> Result<()> {
        if (self.cache_logical_plans || self.cache_compiled_artifacts)
            && (self.max_entries == 0 || self.max_bytes == 0 || self.max_entry_bytes == 0)
        {
            return Err(Diagnostic::error(
                "ENGINE-CACHE-001",
                "enabled caches require non-zero entry, total-byte, and per-entry bounds",
            ));
        }
        if self.max_entry_bytes > self.max_bytes {
            return Err(Diagnostic::error(
                "ENGINE-CACHE-002",
                "per-entry cache limit cannot exceed the total cache byte limit",
            ));
        }
        Ok(())
    }

    /// Returns the shared lifetime for an admitted logical plan.
    #[must_use]
    pub const fn logical_plan_lifetime(self, entry_bytes: u64) -> Option<CacheLifetime> {
        if self.cache_logical_plans && self.admits_size(entry_bytes) {
            Some(CacheLifetime::Shared)
        } else {
            None
        }
    }

    /// Returns the maximum safe lifetime for an admitted compiled artifact.
    #[must_use]
    pub const fn artifact_lifetime(
        self,
        cacheability: ArtifactCacheability,
        entry_bytes: u64,
    ) -> Option<CacheLifetime> {
        if !self.cache_compiled_artifacts || !self.admits_size(entry_bytes) {
            return None;
        }
        match cacheability {
            ArtifactCacheability::Never => None,
            ArtifactCacheability::RequestScoped => Some(CacheLifetime::Request),
            ArtifactCacheability::PlanReusable => Some(CacheLifetime::Shared),
        }
    }

    const fn admits_size(self, entry_bytes: u64) -> bool {
        entry_bytes <= self.max_entry_bytes && entry_bytes <= self.max_bytes
    }
}

impl Default for CachePolicy {
    fn default() -> Self {
        Self {
            cache_logical_plans: true,
            cache_compiled_artifacts: false,
            max_entries: 1_024,
            max_bytes: 64 * 1024 * 1024,
            max_entry_bytes: 4 * 1024 * 1024,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_policy_is_bounded_and_compiled_artifacts_are_opt_in() {
        let policy = CachePolicy::default();
        policy.validate().unwrap();
        assert_eq!(policy.logical_plan_lifetime(1), Some(CacheLifetime::Shared));
        assert_eq!(
            policy.artifact_lifetime(ArtifactCacheability::PlanReusable, 1),
            None
        );
        assert_eq!(policy.logical_plan_lifetime(u64::MAX), None);
    }

    #[test]
    fn request_bound_artifacts_never_gain_shared_lifetime() {
        let policy = CachePolicy {
            cache_compiled_artifacts: true,
            ..CachePolicy::default()
        };
        assert_eq!(
            policy.artifact_lifetime(ArtifactCacheability::RequestScoped, 1),
            Some(CacheLifetime::Request)
        );
        assert_eq!(
            policy.artifact_lifetime(ArtifactCacheability::PlanReusable, 1),
            Some(CacheLifetime::Shared)
        );
        assert_eq!(
            policy.artifact_lifetime(ArtifactCacheability::Never, 1),
            None
        );
    }
}
