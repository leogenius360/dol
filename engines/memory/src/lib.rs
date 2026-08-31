#![forbid(unsafe_code)]
//! Exact in-memory reference engine for the DOL engine SPI.

mod executor;
mod store;
mod transaction;
mod write;

use std::sync::atomic::{AtomicBool, Ordering};

use dol_core::data::DataSet;
use dol_core::diagnostic::{Diagnostic, Result};
use dol_core::expr::Parameters;
use dol_core::model::{Model, ModelDef, RecordView};
use dol_core::ops::{WriteKind, WriteOutcome, WriteSource};
use dol_core::pipeline::Pipeline;
use dol_core::runtime::DynRow;
use dol_engine::{
    Capabilities, Engine, EngineInfo, ExecutionOptions, ExecutionRequest, ExplainPlan,
    PipelineCapabilities, PlacementPolicy, PlanExecutor, Support, TransactionCapabilities,
    WriteCapabilities, WriteExecutor, WriteLimits, WriteRequest, analyze_engine_placement,
};

pub use executor::MemoryStream;
pub use transaction::MemoryTransaction;

use store::{
    Tables, load_dataset, load_dynamic as load_dynamic_dataset, register_model, validate_tables,
};

/// Exact in-memory reference engine and semantic conformance oracle.
#[derive(Debug)]
pub struct MemoryEngine {
    info: EngineInfo,
    capabilities: Capabilities,
    pub(crate) tables: Tables,
    validated: AtomicBool,
}

impl MemoryEngine {
    /// Creates an empty memory engine with its exact Stage-F capability matrix.
    #[must_use]
    pub fn new() -> Self {
        Self {
            info: EngineInfo {
                kind: "memory",
                version: env!("CARGO_PKG_VERSION"),
            },
            capabilities: memory_capabilities(),
            tables: Tables::new(),
            validated: AtomicBool::new(true),
        }
    }

    /// Removes every registered model and materialized row.
    pub fn clear(&mut self) {
        self.tables.clear();
        self.validated.store(true, Ordering::Relaxed);
    }

    /// Registers an empty semantic model definition.
    pub fn register<M>(&mut self) -> Result<()>
    where
        M: Model,
    {
        register_model::<M>(&mut self.tables)?;
        self.invalidate();
        Ok(())
    }

    /// Loads one already-validated concrete model data set into the engine.
    pub fn load<M>(&mut self, data: &DataSet<M>) -> Result<()>
    where
        M: Model + RecordView,
    {
        load_dataset(&mut self.tables, data)?;
        self.invalidate();
        Ok(())
    }

    /// Loads runtime model metadata and dense rows without requiring a Rust model type.
    pub fn load_dynamic(
        &mut self,
        model: ModelDef,
        rows: impl IntoIterator<Item = DynRow>,
    ) -> Result<()> {
        load_dynamic_dataset(&mut self.tables, model, rows.into_iter().collect())?;
        self.invalidate();
        Ok(())
    }

    /// Validates all registered local and cross-model constraints.
    pub fn validate(&self) -> Result<()> {
        if self.validated.load(Ordering::Relaxed) {
            return Ok(());
        }
        validate_tables(&self.tables)?;
        self.validated.store(true, Ordering::Relaxed);
        Ok(())
    }

    /// Borrows the canonical runtime rows currently registered for model `M`.
    pub fn rows<M>(&self) -> Result<&[DynRow]>
    where
        M: Model,
    {
        let model = M::model_def()?;
        let table = self.tables.get(model.key()).ok_or_else(|| {
            Diagnostic::error(
                "MEMORY-STORE-003",
                format!("model `{}` is not registered", model.key().as_str()),
            )
        })?;
        if table.model.fingerprint() != model.fingerprint() {
            return Err(Diagnostic::error(
                "MEMORY-STORE-004",
                "requested Rust model differs from the registered memory model semantics",
            ));
        }
        Ok(table.rows.as_slice())
    }

    /// Lowers and executes one pipeline under explicit parameter and execution policy.
    pub fn execute<T>(
        &self,
        pipeline: &Pipeline<T>,
        parameters: &Parameters,
        options: &ExecutionOptions,
    ) -> Result<MemoryStream> {
        let plan = pipeline.logical_plan()?;
        self.execute_plan(ExecutionRequest {
            plan: &plan,
            parameters,
            options,
        })
    }

    /// Lowers and explains placement for one pipeline without executing it.
    pub fn explain<T>(
        &self,
        pipeline: &Pipeline<T>,
        policy: PlacementPolicy,
    ) -> Result<ExplainPlan> {
        let plan = pipeline.logical_plan()?;
        let placement = analyze_engine_placement(&plan, self, policy)?;
        Ok(ExplainPlan::new(&self.info, &plan, &placement))
    }

    /// Lowers and applies one first-class write atomically.
    pub fn apply<W>(
        &mut self,
        write: &W,
        parameters: &Parameters,
        limits: &WriteLimits,
    ) -> Result<WriteOutcome>
    where
        W: WriteSource,
    {
        let logical = write.logical_write()?;
        self.execute_write(WriteRequest {
            write: &logical,
            parameters,
            limits,
        })
    }

    fn invalidate(&mut self) {
        self.validated.store(false, Ordering::Relaxed);
    }

    pub(crate) fn mark_validated(&self) {
        self.validated.store(true, Ordering::Relaxed);
    }
}

impl Clone for MemoryEngine {
    fn clone(&self) -> Self {
        Self {
            info: self.info.clone(),
            capabilities: self.capabilities.clone(),
            tables: self.tables.clone(),
            validated: AtomicBool::new(self.validated.load(Ordering::Relaxed)),
        }
    }
}

impl Default for MemoryEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine for MemoryEngine {
    fn info(&self) -> &EngineInfo {
        &self.info
    }

    fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }
}

impl PlanExecutor for MemoryEngine {
    type Stream = MemoryStream;

    fn execute_plan(&self, request: ExecutionRequest<'_>) -> Result<Self::Stream> {
        self.validate()?;
        let placement = analyze_engine_placement(request.plan, self, request.options.placement)?;
        if !placement.is_fully_engine() {
            return Err(Diagnostic::error(
                "MEMORY-PLACEMENT-001",
                "memory engine does not execute hybrid local-residual plans",
            ));
        }
        executor::execute(
            &self.tables,
            request.plan,
            request.parameters,
            request.options,
        )
    }
}

impl WriteExecutor for MemoryEngine {
    fn execute_write(&mut self, request: WriteRequest<'_>) -> Result<WriteOutcome> {
        validate_write_request(&self.capabilities, &request)?;
        self.validate()?;

        let outcome = write::apply_atomic_write(
            &mut self.tables,
            request.write,
            request.parameters,
            request.limits,
            validate_tables,
        )?;
        self.mark_validated();
        Ok(outcome)
    }
}

pub(crate) fn validate_write_request(
    capabilities: &Capabilities,
    request: &WriteRequest<'_>,
) -> Result<()> {
    request.limits.validate()?;
    let support = match request.write.kind() {
        WriteKind::Insert => &capabilities.writes.insert,
        WriteKind::InsertMany => &capabilities.writes.insert_many,
        WriteKind::Update => &capabilities.writes.update,
        WriteKind::Delete => &capabilities.writes.delete,
    };
    if !support.is_supported() {
        return Err(Diagnostic::error(
            "MEMORY-WRITE-006",
            format!(
                "memory engine cannot execute this write exactly: {}",
                support.reason().unwrap_or("no exact support")
            ),
        ));
    }
    Ok(())
}

fn memory_capabilities() -> Capabilities {
    Capabilities {
        pipeline: PipelineCapabilities {
            source: Support::ExactNative,
            filter: Support::ExactNative,
            project: Support::ExactNative,
            aggregate: Support::ExactNative,
            unnest: Support::ExactNative,
            window: Support::ExactNative,
            sort: Support::ExactNative,
            distinct: Support::ExactNative,
            slice: Support::ExactNative,
            join: Support::ExactNative,
            set: Support::ExactNative,
            exists: Support::ExactNative,
        },
        writes: WriteCapabilities {
            insert: Support::ExactNative,
            insert_many: Support::ExactNative,
            update: Support::ExactNative,
            delete: Support::ExactNative,
        },
        transaction: TransactionCapabilities {
            transactions: Support::ExactNative,
            atomic_writes: Support::ExactNative,
            referential_integrity: Support::ExactNative,
        },
    }
}
