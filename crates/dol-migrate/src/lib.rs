#![forbid(unsafe_code)]
//! Opt-in migration and catalog subsystem.

pub mod apply;
pub mod catalog;
pub mod diff;
pub mod plan;
pub mod policy;

pub use apply::{
    AppliedStep, ApplyLimits, CatalogMutator, MigrationApplier, MigrationFinish, MigrationReport,
};
pub use catalog::{
    CatalogEntity, CatalogField, CatalogIndex, CatalogInspector, CatalogLimits, CatalogRevision,
    CatalogScope, CatalogSnapshot,
};
pub use diff::{
    CatalogDiff, DiffLimits, MigrationIntent, MigrationOperation, RenameIntent, diff_catalog,
};
pub use plan::{
    AutomationLevel, DataSafety, DefaultMigrationPlanner, MigrationPlan, MigrationPlanIdentity,
    MigrationPlanner, MigrationRisk, MigrationStep, MigrationStepId, OperationalImpact,
};
pub use policy::{MigrationApproval, MigrationPolicy};
