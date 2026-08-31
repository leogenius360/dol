//! Explicit transaction boundary for engines with atomic multi-write semantics.

use dol_core::diagnostic::Result;
use dol_core::ops::WriteOutcome;

use crate::execute::{Engine, WriteRequest};

/// Portable transaction isolation levels currently specified by DOL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum IsolationLevel {
    /// Transaction observes and commits as one serializable unit.
    Serializable,
}

/// Resource limits for a snapshot-based transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransactionLimits {
    /// Maximum rows copied into or retained by the candidate snapshot.
    pub max_snapshot_rows: u64,
    /// Maximum approximate logical row bytes in the candidate snapshot.
    pub max_snapshot_bytes: u64,
    /// Maximum successful writes in one transaction.
    pub max_writes: usize,
}

impl Default for TransactionLimits {
    fn default() -> Self {
        Self {
            max_snapshot_rows: 100_000,
            max_snapshot_bytes: 64 * 1024 * 1024,
            max_writes: 1_024,
        }
    }
}

/// Active engine transaction.
pub trait EngineTransaction {
    /// Applies one logical write to the transaction's private candidate state.
    fn execute_write(&mut self, request: WriteRequest<'_>) -> Result<WriteOutcome>;

    /// Validates and atomically publishes the candidate state.
    fn commit(self) -> Result<()>
    where
        Self: Sized;

    /// Explicitly discards the candidate state.
    fn rollback(self) -> Result<()>
    where
        Self: Sized;
}

/// Engine capable of opening explicit transactions.
pub trait TransactionalEngine: Engine {
    /// Transaction implementation borrowing this engine mutably.
    type Transaction<'a>: EngineTransaction
    where
        Self: 'a;

    /// Starts a transaction under conservative default snapshot limits.
    fn begin_transaction(&mut self, isolation: IsolationLevel) -> Result<Self::Transaction<'_>> {
        self.begin_transaction_with_limits(isolation, TransactionLimits::default())
    }

    /// Starts a transaction under explicit snapshot/write limits.
    fn begin_transaction_with_limits(
        &mut self,
        isolation: IsolationLevel,
        limits: TransactionLimits,
    ) -> Result<Self::Transaction<'_>>;
}
