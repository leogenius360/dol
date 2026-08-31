use dol_core::diagnostic::{Diagnostic, Result};
use dol_core::expr::Parameters;
use dol_core::ops::{WriteOutcome, WriteSource};
use dol_engine::{
    EngineTransaction, IsolationLevel, TransactionLimits, TransactionalEngine, WriteLimits,
    WriteRequest,
};

use crate::store::{Tables, validate_tables};
use crate::{MemoryEngine, validate_write_request, write};

/// Serializable memory transaction over a private candidate snapshot.
pub struct MemoryTransaction<'a> {
    engine: &'a mut MemoryEngine,
    working: Tables,
    limits: TransactionLimits,
    writes: usize,
}

impl<'a> MemoryTransaction<'a> {
    fn new(engine: &'a mut MemoryEngine, limits: TransactionLimits) -> Result<Self> {
        validate_snapshot_limits(&engine.tables, limits)?;
        Ok(Self {
            working: engine.tables.clone(),
            engine,
            limits,
            writes: 0,
        })
    }

    /// Lowers and applies one typed write to this transaction's private candidate state.
    pub fn apply<W>(
        &mut self,
        operation: &W,
        parameters: &Parameters,
        limits: &WriteLimits,
    ) -> Result<WriteOutcome>
    where
        W: WriteSource,
    {
        let write = operation.logical_write()?;
        self.execute_write(WriteRequest {
            write: &write,
            parameters,
            limits,
        })
    }
}

impl EngineTransaction for MemoryTransaction<'_> {
    fn execute_write(&mut self, request: WriteRequest<'_>) -> Result<WriteOutcome> {
        if self.writes >= self.limits.max_writes {
            return Err(Diagnostic::error(
                "MEMORY-TX-002",
                "transaction exceeds the configured successful-write limit",
            ));
        }
        validate_write_request(&self.engine.capabilities, &request)?;
        let transaction_limits = self.limits;
        let outcome = write::apply_atomic_write(
            &mut self.working,
            request.write,
            request.parameters,
            request.limits,
            |tables| validate_snapshot_limits(tables, transaction_limits),
        )?;
        self.writes += 1;
        Ok(outcome)
    }

    fn commit(self) -> Result<()> {
        validate_snapshot_limits(&self.working, self.limits)?;
        validate_tables(&self.working)?;
        self.engine.tables = self.working;
        self.engine.mark_validated();
        Ok(())
    }

    fn rollback(self) -> Result<()> {
        Ok(())
    }
}

impl TransactionalEngine for MemoryEngine {
    type Transaction<'a>
        = MemoryTransaction<'a>
    where
        Self: 'a;

    fn begin_transaction_with_limits(
        &mut self,
        isolation: IsolationLevel,
        limits: TransactionLimits,
    ) -> Result<Self::Transaction<'_>> {
        if isolation != IsolationLevel::Serializable {
            return Err(Diagnostic::error(
                "MEMORY-TX-001",
                "memory engine only exposes DOL serializable transactions",
            ));
        }
        self.validate()?;
        if limits.max_snapshot_rows == 0 || limits.max_snapshot_bytes == 0 || limits.max_writes == 0
        {
            return Err(Diagnostic::error(
                "MEMORY-TX-002",
                "transaction snapshot and write limits must be non-zero",
            ));
        }
        MemoryTransaction::new(self, limits)
    }
}

fn validate_snapshot_limits(tables: &Tables, limits: TransactionLimits) -> Result<()> {
    let mut rows = 0_u64;
    let mut bytes = 0_u64;
    for table in tables.values() {
        rows = rows.saturating_add(u64::try_from(table.rows.len()).unwrap_or(u64::MAX));
        if rows > limits.max_snapshot_rows {
            return Err(Diagnostic::error(
                "MEMORY-TX-003",
                "transaction snapshot exceeds the configured row limit",
            ));
        }
        for row in &table.rows {
            bytes = bytes.saturating_add(row.logical_bytes());
            if bytes > limits.max_snapshot_bytes {
                return Err(Diagnostic::error(
                    "MEMORY-TX-004",
                    "transaction snapshot exceeds the configured logical-byte limit",
                ));
            }
        }
    }
    Ok(())
}
