#![forbid(unsafe_code)]
//! Exact PostgreSQL mapping, compilation, and bounded read execution for DOL.
//!
//! [`PostgresEngine::new`] remains a purely offline compiler boundary and opens
//! no network resources. [`PostgresEngine::with_runtime`] enables the Stage H
//! read runtime over TLS-required PostgreSQL connections. Every execution checks
//! the live server/session and mapped table contract before advertising results.

mod codec;
mod compiler;
mod inspect;
mod mapping;
mod runtime;
mod sql;

use dol_core::diagnostic::Result;
use dol_core::expr::Parameters;
use dol_core::pipeline::Pipeline;
use dol_core::plan::{LogicalNode, LogicalPlan};
use dol_engine::{
    Capabilities, Engine, EngineInfo, ExecutionOptions, ExecutionRequest, ExplainPlan,
    PlacementPolicy, PlanExecutor, Support, analyze_engine_placement,
};

pub use compiler::PostgresCompiler;
pub use mapping::{ColumnMapping, PostgresCatalog, TableMapping};
pub use runtime::{PostgresRuntimeConfig, PostgresStream};
pub use sql::{CompiledQuery, SqlBind, SqlOutputColumn};

/// PostgreSQL adapter configuration, compiler, and optional read runtime.
#[derive(Debug, Clone)]
pub struct PostgresEngine {
    info: EngineInfo,
    capabilities: Capabilities,
    catalog: PostgresCatalog,
    runtime: Option<PostgresRuntimeConfig>,
}

impl PostgresEngine {
    /// Creates an offline PostgreSQL adapter over explicit physical mappings.
    ///
    /// Offline adapters advertise no execution capabilities and never open a
    /// database connection.
    #[must_use]
    pub fn new(catalog: PostgresCatalog) -> Self {
        Self {
            info: EngineInfo {
                kind: "postgres",
                version: env!("CARGO_PKG_VERSION"),
            },
            capabilities: Capabilities::default(),
            catalog,
            runtime: None,
        }
    }

    /// Creates a TLS-required PostgreSQL read runtime over explicit mappings.
    #[must_use]
    pub fn with_runtime(catalog: PostgresCatalog, runtime: PostgresRuntimeConfig) -> Self {
        Self {
            info: EngineInfo {
                kind: "postgres",
                version: env!("CARGO_PKG_VERSION"),
            },
            capabilities: runtime::runtime_capabilities(),
            catalog,
            runtime: Some(runtime),
        }
    }

    /// Physical model/table catalog.
    #[must_use]
    pub const fn catalog(&self) -> &PostgresCatalog {
        &self.catalog
    }

    /// Runtime connection configuration, when execution was explicitly enabled.
    #[must_use]
    pub const fn runtime_config(&self) -> Option<&PostgresRuntimeConfig> {
        self.runtime.as_ref()
    }

    /// Creates an exact SQL compiler without opening network/database resources.
    #[must_use]
    pub const fn compiler(&self) -> PostgresCompiler<'_> {
        PostgresCompiler::new(&self.catalog)
    }

    /// Compiles one logical plan without opening network/database resources.
    pub fn compile(&self, plan: &LogicalPlan, parameters: &Parameters) -> Result<CompiledQuery> {
        self.compiler().compile(plan, parameters)
    }

    /// Lowers and executes one pipeline through the TLS-required read runtime.
    pub fn execute<T>(
        &self,
        pipeline: &Pipeline<T>,
        parameters: &Parameters,
        options: &ExecutionOptions,
    ) -> Result<PostgresStream> {
        let plan = pipeline.prepared_plan()?;
        self.execute_plan(ExecutionRequest {
            plan: &plan,
            parameters,
            options,
        })
    }

    /// Lowers and explains PostgreSQL runtime placement without opening a connection.
    pub fn explain<T>(
        &self,
        pipeline: &Pipeline<T>,
        policy: PlacementPolicy,
    ) -> Result<ExplainPlan> {
        let plan = pipeline.prepared_plan()?;
        self.explain_plan(&plan, policy)
    }

    /// Explains PostgreSQL placement for an already lowered logical plan.
    ///
    /// This separates semantic pipeline preparation from placement and explain
    /// construction for callers that reuse one immutable plan.
    pub fn explain_plan(&self, plan: &LogicalPlan, policy: PlacementPolicy) -> Result<ExplainPlan> {
        let placement = analyze_engine_placement(plan, self, policy)?;
        Ok(ExplainPlan::new(&self.info, plan, &placement))
    }
}

impl Engine for PostgresEngine {
    fn info(&self) -> &EngineInfo {
        &self.info
    }

    fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }

    fn support_for_node(&self, node: &LogicalNode) -> Support {
        let base = dol_engine::placement::capability_support_for_node(node, &self.capabilities);
        if base.is_supported() && !self.compiler().supports_node(node) {
            Support::unsupported(
                "concrete PostgreSQL node uses a mapping/type/expression outside the exact compiler surface",
            )
        } else {
            base
        }
    }
}
