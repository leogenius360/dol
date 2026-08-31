#![forbid(unsafe_code)]
//! Engine-author SPI, exact capability analysis, placement, and execution streams.

pub mod cache;
pub mod capability;
pub mod execute;
pub mod explain;
pub mod mapping;
pub mod placement;
pub mod stream;
pub mod transaction;

pub mod prelude {
    //! Common engine-author and application execution imports.

    pub use crate::cache::{ArtifactCacheability, CacheLifetime, CachePolicy};
    pub use crate::capability::{
        Capabilities, PipelineCapabilities, Support, TransactionCapabilities, WriteCapabilities,
    };
    pub use crate::execute::{
        Engine, EngineInfo, ExecutionLimits, ExecutionOptions, ExecutionRequest, PlanExecutor,
        WriteExecutor, WriteLimits, WriteRequest,
    };
    pub use crate::placement::{
        PlacementPolicy, PlacementSite, TransferBudget, analyze_engine_placement, analyze_placement,
    };
    pub use crate::stream::{
        CollectLimits, DataStream, ExecutionBatch, ExecutionRow, collect_stream,
        collect_stream_with_limits,
    };
    pub use crate::transaction::{
        EngineTransaction, IsolationLevel, TransactionLimits, TransactionalEngine,
    };
}

pub use dol_core::expr::{BoundParameter, Parameters};

pub use cache::{ArtifactCacheability, CacheLifetime, CachePolicy};
pub use capability::{
    Capabilities, PipelineCapabilities, Support, TransactionCapabilities, WriteCapabilities,
};
pub use execute::{
    Engine, EngineInfo, ExecutionLimits, ExecutionOptions, ExecutionRequest, PlanExecutor,
    WriteExecutor, WriteLimits, WriteRequest,
};
pub use explain::{ExplainPlan, ExplainStep};
pub use placement::{
    NodePlacement, PlacementPlan, PlacementPolicy, PlacementSite, TransferBudget,
    analyze_engine_placement, analyze_placement,
};
pub use stream::{
    CollectLimits, DataStream, ExecutionBatch, ExecutionRow, collect_stream,
    collect_stream_with_limits,
};
pub use transaction::{EngineTransaction, IsolationLevel, TransactionLimits, TransactionalEngine};
