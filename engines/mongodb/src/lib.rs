#![forbid(unsafe_code)]
//! Exact MongoDB mapping, offline compilation, and bounded pull execution.

mod codec;
mod compiled;
mod compiler;
mod mapping;
mod runtime;

use dol_core::diagnostic::Result;
use dol_core::expr::Parameters;
use dol_core::pipeline::Pipeline;
use dol_core::plan::{LogicalNode, LogicalPlan};
use dol_engine::{
    Capabilities, Engine, EngineInfo, ExecutionOptions, ExecutionRequest, ExplainPlan,
    PlacementPolicy, PlanExecutor, Support, analyze_engine_placement,
};

pub use compiled::{BsonOutputField, CompiledAggregation};
pub use compiler::MongodbCompiler;
pub use mapping::{CollectionMapping, FieldMapping, MongodbCatalog};
pub use runtime::{MongodbRuntimeConfig, MongodbStream};

/// MongoDB adapter configured for offline compilation or live bounded reads.
#[derive(Debug, Clone)]
pub struct MongodbEngine {
    info: EngineInfo,
    capabilities: Capabilities,
    catalog: MongodbCatalog,
    runtime: Option<MongodbRuntimeConfig>,
}

impl MongodbEngine {
    /// Creates an offline adapter. It advertises no executable capabilities and
    /// opens no network resources.
    #[must_use]
    pub fn new(catalog: MongodbCatalog) -> Self {
        Self {
            info: EngineInfo {
                kind: "mongodb",
                version: env!("CARGO_PKG_VERSION"),
            },
            capabilities: Capabilities::default(),
            catalog,
            runtime: None,
        }
    }

    /// Creates an explicitly enabled bounded read runtime.
    #[must_use]
    pub fn with_runtime(catalog: MongodbCatalog, runtime: MongodbRuntimeConfig) -> Self {
        Self {
            info: EngineInfo {
                kind: "mongodb",
                version: env!("CARGO_PKG_VERSION"),
            },
            capabilities: runtime::runtime_capabilities(),
            catalog,
            runtime: Some(runtime),
        }
    }

    /// Physical semantic-model to collection registry.
    #[must_use]
    pub const fn catalog(&self) -> &MongodbCatalog {
        &self.catalog
    }

    /// Runtime connection policy, when live execution was explicitly enabled.
    #[must_use]
    pub const fn runtime_config(&self) -> Option<&MongodbRuntimeConfig> {
        self.runtime.as_ref()
    }

    /// Creates a pure offline compiler.
    #[must_use]
    pub const fn compiler(&self) -> MongodbCompiler<'_> {
        MongodbCompiler::new(&self.catalog)
    }

    /// Compiles one logical plan without opening a connection.
    pub fn compile(
        &self,
        plan: &LogicalPlan,
        parameters: &Parameters,
    ) -> Result<CompiledAggregation> {
        self.compiler().compile(plan, parameters)
    }

    /// Lowers and executes one typed pipeline.
    pub fn execute<T>(
        &self,
        pipeline: &Pipeline<T>,
        parameters: &Parameters,
        options: &ExecutionOptions,
    ) -> Result<MongodbStream> {
        let plan = pipeline.prepared_plan()?;
        self.execute_plan(ExecutionRequest {
            plan: &plan,
            parameters,
            options,
        })
    }

    /// Explains coarse operator placement without opening a connection.
    pub fn explain<T>(
        &self,
        pipeline: &Pipeline<T>,
        policy: PlacementPolicy,
    ) -> Result<ExplainPlan> {
        let plan = pipeline.prepared_plan()?;
        let placement = analyze_engine_placement(&plan, self, policy)?;
        Ok(ExplainPlan::new(&self.info, &plan, &placement))
    }
}

impl Default for MongodbEngine {
    fn default() -> Self {
        Self::new(MongodbCatalog::new())
    }
}

impl Engine for MongodbEngine {
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
                "concrete MongoDB node uses a mapping/type/expression outside the exact compiler surface",
            )
        } else {
            base
        }
    }
}
