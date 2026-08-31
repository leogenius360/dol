#![forbid(unsafe_code)]
//! Bounded incremental JSON Lines execution engine for DOL.
//!
//! The adapter deliberately advertises only semantics it can execute exactly while
//! reading one file incrementally. Materializing/global and multi-source operators
//! remain unsupported rather than being approximated.

mod decode;
mod source;
mod stream;

use std::path::Path;

use dol_core::diagnostic::{Diagnostic, Result};
use dol_core::expr::Parameters;
use dol_core::model::{Model, ModelDef};
use dol_core::pipeline::Pipeline;
use dol_engine::{
    Capabilities, Engine, EngineInfo, ExecutionOptions, ExecutionRequest, ExplainPlan,
    PipelineCapabilities, PlacementPolicy, PlanExecutor, Support, TransactionCapabilities,
    WriteCapabilities, analyze_engine_placement,
};

pub use stream::JsonlStream;

use source::{Sources, bind_definition, bind_model, canonical_root};

const MAX_JSON_NESTING_DEPTH: usize = 64;

/// Hard adapter-local bounds for untrusted JSONL file input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonlLimits {
    /// Maximum bytes in one bound source file.
    pub max_source_bytes: u64,
    /// Maximum UTF-8 bytes in one physical JSONL record, excluding LF.
    pub max_line_bytes: usize,
    /// Maximum source rows scanned by one execution.
    pub max_scanned_rows: u64,
    /// Maximum nested DOL list/map/tuple/record depth accepted from one JSON value.
    ///
    /// Values above 64 are rejected so the adapter never promises a depth beyond
    /// its JSON parser safety envelope.
    pub max_nesting_depth: usize,
}

impl JsonlLimits {
    /// Validates that every input boundary is finite and non-zero.
    pub fn validate(&self) -> Result<()> {
        if self.max_source_bytes == 0
            || self.max_line_bytes == 0
            || self.max_scanned_rows == 0
            || self.max_nesting_depth == 0
        {
            return Err(Diagnostic::error(
                "JSONL-LIMIT-000",
                "JSONL source, line, row, and nesting limits must all be greater than zero",
            ));
        }
        if self.max_nesting_depth > MAX_JSON_NESTING_DEPTH {
            return Err(Diagnostic::error(
                "JSONL-LIMIT-000",
                format!("JSONL nesting depth may not exceed {MAX_JSON_NESTING_DEPTH}"),
            ));
        }
        Ok(())
    }
}

impl Default for JsonlLimits {
    fn default() -> Self {
        Self {
            max_source_bytes: 1024 * 1024 * 1024,
            max_line_bytes: 8 * 1024 * 1024,
            max_scanned_rows: 10_000_000,
            max_nesting_depth: 64,
        }
    }
}

/// Read-only JSONL engine rooted at one filesystem directory.
#[derive(Debug, Clone)]
pub struct JsonlEngine {
    info: EngineInfo,
    capabilities: Capabilities,
    root: std::path::PathBuf,
    sources: Sources,
    limits: JsonlLimits,
}

impl JsonlEngine {
    /// Opens a filesystem root. Bound sources may not escape this directory.
    pub fn new(root: impl AsRef<Path>) -> Result<Self> {
        Self::with_limits(root, JsonlLimits::default())
    }

    /// Opens a filesystem root with explicit hard input limits.
    pub fn with_limits(root: impl AsRef<Path>, limits: JsonlLimits) -> Result<Self> {
        limits.validate()?;
        Ok(Self {
            info: EngineInfo {
                kind: "jsonl",
                version: env!("CARGO_PKG_VERSION"),
            },
            capabilities: jsonl_capabilities(),
            root: canonical_root(root)?,
            sources: Sources::new(),
            limits,
        })
    }

    /// Binds one Rust model to a JSONL file relative to the configured root.
    pub fn bind<M>(&mut self, relative_path: impl AsRef<Path>) -> Result<()>
    where
        M: Model,
    {
        bind_model::<M>(&mut self.sources, &self.root, relative_path)
    }

    /// Binds runtime model metadata to a JSONL file relative to the configured root.
    pub fn bind_dynamic(&mut self, model: ModelDef, relative_path: impl AsRef<Path>) -> Result<()> {
        bind_definition(&mut self.sources, &self.root, model, relative_path)
    }

    /// Adapter-local untrusted-input limits.
    #[must_use]
    pub const fn limits(&self) -> &JsonlLimits {
        &self.limits
    }

    /// Lowers and executes one incrementally supported pipeline.
    pub fn execute<T>(
        &self,
        pipeline: &Pipeline<T>,
        parameters: &Parameters,
        options: &ExecutionOptions,
    ) -> Result<JsonlStream> {
        let plan = pipeline.logical_plan()?;
        self.execute_plan(ExecutionRequest {
            plan: &plan,
            parameters,
            options,
        })
    }

    /// Lowers and explains exact JSONL placement without reading the file.
    pub fn explain<T>(
        &self,
        pipeline: &Pipeline<T>,
        policy: PlacementPolicy,
    ) -> Result<ExplainPlan> {
        let plan = pipeline.logical_plan()?;
        let placement = analyze_engine_placement(&plan, self, policy)?;
        Ok(ExplainPlan::new(&self.info, &plan, &placement))
    }
}

impl Engine for JsonlEngine {
    fn info(&self) -> &EngineInfo {
        &self.info
    }

    fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }
}

impl PlanExecutor for JsonlEngine {
    type Stream = JsonlStream;

    fn execute_plan(&self, request: ExecutionRequest<'_>) -> Result<Self::Stream> {
        let placement = analyze_engine_placement(request.plan, self, request.options.placement)?;
        if !placement.is_fully_engine() {
            return Err(Diagnostic::error(
                "JSONL-PLACEMENT-001",
                "JSONL engine does not execute hybrid local-residual plans",
            ));
        }
        JsonlStream::open(
            &self.sources,
            request.plan,
            request.parameters,
            request.options,
            &self.limits,
        )
    }
}

fn jsonl_capabilities() -> Capabilities {
    let materializing = || {
        Support::unsupported(
            "bounded incremental JSONL execution does not yet implement this materializing operator",
        )
    };
    let multi_source = || {
        Support::unsupported(
            "bounded incremental JSONL execution does not yet implement multi-source execution",
        )
    };
    let writes = || Support::unsupported("Stage G JSONL execution is read-only");
    Capabilities {
        pipeline: PipelineCapabilities {
            source: Support::ExactNative,
            filter: Support::ExactNative,
            project: Support::ExactNative,
            aggregate: materializing(),
            unnest: Support::ExactNative,
            window: materializing(),
            sort: materializing(),
            distinct: materializing(),
            slice: Support::ExactNative,
            join: multi_source(),
            set: multi_source(),
            exists: multi_source(),
        },
        writes: WriteCapabilities {
            insert: writes(),
            insert_many: writes(),
            update: writes(),
            delete: writes(),
        },
        transaction: TransactionCapabilities {
            transactions: writes(),
            atomic_writes: writes(),
            referential_integrity: Support::unsupported(
                "read-only JSONL execution does not own a multi-model transactional store",
            ),
        },
    }
}
