//! Revision-bound migration plans and independent risk dimensions.

use dol_core::diagnostic::{Diagnostic, Result};

use crate::catalog::{CatalogRevision, CatalogScope, CatalogSnapshot};
use crate::diff::{CatalogDiff, DiffLimits, MigrationIntent, MigrationOperation, diff_catalog};

/// Data-level safety of a migration change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DataSafety {
    /// No expected data loss.
    Safe,
    /// Existing data must be rewritten.
    RequiresDataRewrite,
    /// The operation can destroy information.
    Destructive,
}

/// Operational impact of a migration change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OperationalImpact {
    /// Catalog-only or metadata-only change.
    MetadataOnly,
    /// May lock affected data.
    Locking,
    /// Rewrites stored data.
    DataRewrite,
    /// Rebuilds an external structure such as an index.
    ExternalRebuild,
}

/// Automation policy for a migration change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AutomationLevel {
    /// May be applied automatically under policy.
    Automatic,
    /// Requires explicit approval.
    RequiresApproval,
    /// Must be performed manually.
    Manual,
    /// Not supported by this engine.
    Unsupported,
}

/// Independent migration risk axes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MigrationRisk {
    /// Data safety.
    pub data: DataSafety,
    /// Runtime/operational impact.
    pub operation: OperationalImpact,
    /// Automation level.
    pub automation: AutomationLevel,
}

impl MigrationRisk {
    /// Conservative minimum risk for a generic catalog operation.
    #[must_use]
    pub const fn for_operation(operation: &MigrationOperation) -> Self {
        match operation {
            MigrationOperation::CreateEntity { .. } => Self {
                data: DataSafety::Safe,
                operation: OperationalImpact::Locking,
                automation: AutomationLevel::Automatic,
            },
            MigrationOperation::DropEntity { .. } | MigrationOperation::DropField { .. } => Self {
                data: DataSafety::Destructive,
                operation: OperationalImpact::Locking,
                automation: AutomationLevel::RequiresApproval,
            },
            MigrationOperation::RenameEntity { .. }
            | MigrationOperation::RenameField { .. }
            | MigrationOperation::RenameIndex { .. } => Self {
                data: DataSafety::Safe,
                operation: OperationalImpact::MetadataOnly,
                automation: AutomationLevel::RequiresApproval,
            },
            MigrationOperation::AddField { field, .. } if !field.required() => Self {
                data: DataSafety::Safe,
                operation: OperationalImpact::MetadataOnly,
                automation: AutomationLevel::Automatic,
            },
            MigrationOperation::AddField { .. } | MigrationOperation::AlterField { .. } => Self {
                data: DataSafety::RequiresDataRewrite,
                operation: OperationalImpact::DataRewrite,
                automation: AutomationLevel::RequiresApproval,
            },
            MigrationOperation::CreateIndex { .. } => Self {
                data: DataSafety::Safe,
                operation: OperationalImpact::ExternalRebuild,
                automation: AutomationLevel::RequiresApproval,
            },
            MigrationOperation::DropIndex { .. } => Self {
                data: DataSafety::Safe,
                operation: OperationalImpact::Locking,
                automation: AutomationLevel::RequiresApproval,
            },
        }
    }

    fn covers(self, minimum: Self) -> bool {
        self.data >= minimum.data
            && operational_rank(self.operation) >= operational_rank(minimum.operation)
            && self.automation >= minimum.automation
    }
}

/// Stable identifier of one ordered plan step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MigrationStepId(u32);

impl MigrationStepId {
    /// One-based step sequence number.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// One typed operation and its independently classified risks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationStep {
    id: MigrationStepId,
    operation: MigrationOperation,
    risk: MigrationRisk,
}

impl MigrationStep {
    /// Step identifier.
    #[must_use]
    pub const fn id(&self) -> MigrationStepId {
        self.id
    }

    /// Typed catalog operation.
    #[must_use]
    pub const fn operation(&self) -> &MigrationOperation {
        &self.operation
    }

    /// Independent risk classification.
    #[must_use]
    pub const fn risk(&self) -> MigrationRisk {
        self.risk
    }
}

/// Complete identity to which approvals are bound.
///
/// This intentionally stores the exact ordered steps rather than relying on a
/// collision-prone locally invented hash algorithm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationPlanIdentity {
    scope: CatalogScope,
    source_revision: CatalogRevision,
    target_revision: CatalogRevision,
    steps: Vec<MigrationStep>,
}

impl MigrationPlanIdentity {
    /// Scope bound into the identity.
    #[must_use]
    pub const fn scope(&self) -> &CatalogScope {
        &self.scope
    }

    /// Source revision bound into the identity.
    #[must_use]
    pub const fn source_revision(&self) -> &CatalogRevision {
        &self.source_revision
    }

    /// Target revision bound into the identity.
    #[must_use]
    pub const fn target_revision(&self) -> &CatalogRevision {
        &self.target_revision
    }

    /// Exact ordered steps bound into the identity.
    #[must_use]
    pub fn steps(&self) -> &[MigrationStep] {
        &self.steps
    }
}

/// A catalog-preconditioned, target-bearing migration plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationPlan {
    identity: MigrationPlanIdentity,
    source: CatalogSnapshot,
    target: CatalogSnapshot,
}

impl MigrationPlan {
    /// Builds a plan from an already validated catalog diff.
    pub fn from_diff(diff: CatalogDiff) -> Result<Self> {
        let steps = diff
            .operations()
            .iter()
            .cloned()
            .enumerate()
            .map(|(index, operation)| {
                let sequence = index.checked_add(1).ok_or_else(|| {
                    Diagnostic::error("MIGRATE-PLAN-001", "migration step count overflowed")
                })?;
                let sequence = u32::try_from(sequence).map_err(|_| {
                    Diagnostic::error(
                        "MIGRATE-PLAN-001",
                        "migration step count exceeds the supported identifier range",
                    )
                })?;
                let risk = MigrationRisk::for_operation(&operation);
                Ok(MigrationStep {
                    id: MigrationStepId(sequence),
                    operation,
                    risk,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Self::try_new(diff.source().clone(), diff.target().clone(), steps)
    }

    /// Creates a plan with engine-refined risk classifications.
    ///
    /// A backend may elevate any risk axis, but cannot classify an operation
    /// below the generic conservative minimum.
    pub fn try_new(
        source: CatalogSnapshot,
        target: CatalogSnapshot,
        steps: Vec<MigrationStep>,
    ) -> Result<Self> {
        if source.scope() != target.scope() {
            return Err(Diagnostic::error(
                "MIGRATE-PLAN-002",
                "migration plan source and target scopes differ",
            ));
        }
        for (index, step) in steps.iter().enumerate() {
            let expected = u32::try_from(index + 1).map_err(|_| {
                Diagnostic::error("MIGRATE-PLAN-001", "migration step count overflowed")
            })?;
            if step.id.0 != expected {
                return Err(Diagnostic::error(
                    "MIGRATE-PLAN-003",
                    "migration step identifiers must be contiguous and one-based",
                ));
            }
            let minimum = MigrationRisk::for_operation(&step.operation);
            if !step.risk.covers(minimum) {
                return Err(Diagnostic::error(
                    "MIGRATE-PLAN-004",
                    format!("step {} understates its generic minimum risk", step.id.0),
                ));
            }
        }
        let identity = MigrationPlanIdentity {
            scope: source.scope().clone(),
            source_revision: source.revision().clone(),
            target_revision: target.revision().clone(),
            steps,
        };
        Ok(Self {
            identity,
            source,
            target,
        })
    }

    /// Complete approval identity.
    #[must_use]
    pub const fn identity(&self) -> &MigrationPlanIdentity {
        &self.identity
    }

    /// Exact catalog scope.
    #[must_use]
    pub const fn scope(&self) -> &CatalogScope {
        self.identity.scope()
    }

    /// Required source revision.
    #[must_use]
    pub const fn source_revision(&self) -> &CatalogRevision {
        self.identity.source_revision()
    }

    /// Expected target revision.
    #[must_use]
    pub const fn target_revision(&self) -> &CatalogRevision {
        self.identity.target_revision()
    }

    /// Exact source snapshot used for planning.
    #[must_use]
    pub const fn source(&self) -> &CatalogSnapshot {
        &self.source
    }

    /// Exact expected catalog after application.
    #[must_use]
    pub const fn target(&self) -> &CatalogSnapshot {
        &self.target
    }

    /// Ordered typed steps.
    #[must_use]
    pub fn steps(&self) -> &[MigrationStep] {
        self.identity.steps()
    }

    /// Compatibility-oriented human-readable change descriptions.
    pub fn changes(&self) -> impl ExactSizeIterator<Item = String> + '_ {
        self.steps().iter().map(|step| step.operation.summary())
    }

    /// Whether the source catalog already has the target definition.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.steps().is_empty()
    }
}

/// Produces a revision-bound typed plan.
pub trait MigrationPlanner {
    /// Plans the transition without accessing or mutating a backend.
    fn plan(
        &self,
        source: &CatalogSnapshot,
        target: &CatalogSnapshot,
        intent: &MigrationIntent,
    ) -> Result<MigrationPlan>;
}

/// Deterministic backend-neutral planner.
#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultMigrationPlanner {
    limits: DiffLimits,
}

impl DefaultMigrationPlanner {
    /// Creates a planner with explicit diff limits.
    pub fn try_new(limits: DiffLimits) -> Result<Self> {
        if limits.max_operations == 0 || limits.max_rename_intents == 0 {
            return Err(Diagnostic::error(
                "MIGRATE-PLAN-005",
                "planner limits must be greater than zero",
            ));
        }
        Ok(Self { limits })
    }

    /// Active diff limits.
    #[must_use]
    pub const fn limits(&self) -> DiffLimits {
        self.limits
    }
}

impl MigrationPlanner for DefaultMigrationPlanner {
    fn plan(
        &self,
        source: &CatalogSnapshot,
        target: &CatalogSnapshot,
        intent: &MigrationIntent,
    ) -> Result<MigrationPlan> {
        MigrationPlan::from_diff(diff_catalog(source, target, intent, self.limits)?)
    }
}

const fn operational_rank(impact: OperationalImpact) -> u8 {
    match impact {
        OperationalImpact::MetadataOnly => 0,
        OperationalImpact::Locking => 1,
        OperationalImpact::ExternalRebuild => 2,
        OperationalImpact::DataRewrite => 3,
    }
}

#[cfg(test)]
mod tests {
    use crate::catalog::{
        CatalogEntity, CatalogField, CatalogRevision, CatalogScope, CatalogSnapshot,
    };

    use super::*;

    fn snapshot(revision: &str, fields: Vec<CatalogField>) -> CatalogSnapshot {
        CatalogSnapshot::try_new(
            CatalogScope::try_new("fake", "test").unwrap(),
            CatalogRevision::try_new(revision).unwrap(),
            vec![CatalogEntity::try_new("user", "users", fields, Vec::new()).unwrap()],
        )
        .unwrap()
    }

    #[test]
    fn planner_emits_typed_ordered_risk_classified_steps() {
        let source = snapshot(
            "one",
            vec![CatalogField::try_new("id", "id", "u64", true, false).unwrap()],
        );
        let target = snapshot(
            "two",
            vec![
                CatalogField::try_new("id", "id", "u64", true, false).unwrap(),
                CatalogField::try_new("nickname", "nickname", "text", false, true).unwrap(),
            ],
        );
        let plan = DefaultMigrationPlanner::default()
            .plan(&source, &target, &MigrationIntent::default())
            .unwrap();
        assert_eq!(plan.steps().len(), 1);
        assert_eq!(plan.steps()[0].id().get(), 1);
        assert_eq!(plan.steps()[0].risk().data, DataSafety::Safe);
        assert_eq!(
            plan.steps()[0].risk().automation,
            AutomationLevel::Automatic
        );
        assert_eq!(
            plan.changes().collect::<Vec<_>>(),
            ["add field `nickname` to `user`"]
        );
    }
}
