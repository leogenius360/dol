//! Structured exact-semantic engine capabilities.

/// Semantic support for one operation family.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Support {
    /// The backend natively reproduces DOL semantics.
    ExactNative,
    /// The adapter reproduces DOL semantics by exact compensation/emulation.
    ExactEmulated,
    /// The requested semantics are unavailable.
    Unsupported {
        /// Human-readable reason suitable for placement/explain output.
        reason: String,
    },
}

impl Support {
    /// Creates an unsupported capability with an explanation.
    #[must_use]
    pub fn unsupported(reason: impl Into<String>) -> Self {
        Self::Unsupported {
            reason: reason.into(),
        }
    }

    /// Whether this capability can reproduce DOL semantics exactly.
    #[must_use]
    pub const fn is_supported(&self) -> bool {
        matches!(self, Self::ExactNative | Self::ExactEmulated)
    }

    /// Whether the operation is exact and native to the engine.
    #[must_use]
    pub const fn is_native(&self) -> bool {
        matches!(self, Self::ExactNative)
    }

    /// Unsupported reason, when applicable.
    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Unsupported { reason } => Some(reason),
            Self::ExactNative | Self::ExactEmulated => None,
        }
    }
}

/// Capability matrix for logical pipeline nodes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineCapabilities {
    /// Semantic model sources.
    pub source: Support,
    /// Truth-valued filtering without existential dependencies.
    pub filter: Support,
    /// Scalar/tuple/record projection.
    pub project: Support,
    /// Global and grouped aggregation.
    pub aggregate: Support,
    /// List unnesting.
    pub unnest: Support,
    /// Window computation.
    pub window: Support,
    /// Semantic ordering.
    pub sort: Support,
    /// Duplicate elimination.
    pub distinct: Support,
    /// Offset/limit slicing.
    pub slice: Support,
    /// Join execution.
    pub join: Support,
    /// Set algebra.
    pub set: Support,
    /// Existential pipeline dependencies, including correlation.
    pub exists: Support,
}

impl Default for PipelineCapabilities {
    fn default() -> Self {
        let unsupported = || Support::unsupported("pipeline capability is not implemented");
        Self {
            source: unsupported(),
            filter: unsupported(),
            project: unsupported(),
            aggregate: unsupported(),
            unnest: unsupported(),
            window: unsupported(),
            sort: unsupported(),
            distinct: unsupported(),
            slice: unsupported(),
            join: unsupported(),
            set: unsupported(),
            exists: unsupported(),
        }
    }
}

/// Capability matrix for first-class writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteCapabilities {
    /// Single-row inserts.
    pub insert: Support,
    /// Bulk inserts.
    pub insert_many: Support,
    /// Simultaneous updates.
    pub update: Support,
    /// Deletes.
    pub delete: Support,
}

impl Default for WriteCapabilities {
    fn default() -> Self {
        let unsupported = || Support::unsupported("write capability is not implemented");
        Self {
            insert: unsupported(),
            insert_many: unsupported(),
            update: unsupported(),
            delete: unsupported(),
        }
    }
}

/// Transaction and multi-model integrity capabilities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactionCapabilities {
    /// Explicit transaction boundary.
    pub transactions: Support,
    /// Atomic commit/rollback for a transaction's writes.
    pub atomic_writes: Support,
    /// Cross-model referential-integrity validation.
    pub referential_integrity: Support,
}

impl Default for TransactionCapabilities {
    fn default() -> Self {
        let unsupported = || Support::unsupported("transaction capability is not implemented");
        Self {
            transactions: unsupported(),
            atomic_writes: unsupported(),
            referential_integrity: unsupported(),
        }
    }
}

/// Structured engine capability descriptor.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Capabilities {
    /// Logical pipeline support.
    pub pipeline: PipelineCapabilities,
    /// First-class write support.
    pub writes: WriteCapabilities,
    /// Transaction/integrity support.
    pub transaction: TransactionCapabilities,
}
