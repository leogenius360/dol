//! Explicit migration policy and plan-bound approvals.

use std::collections::BTreeSet;

use dol_core::diagnostic::{Diagnostic, Result};

use crate::plan::{
    AutomationLevel, DataSafety, MigrationPlan, MigrationPlanIdentity, MigrationStepId,
    OperationalImpact,
};

/// Explicit opt-ins controlling which risks may be applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MigrationPolicy {
    allow_data_rewrite: bool,
    allow_destructive: bool,
    allow_locking: bool,
    allow_external_rebuild: bool,
}

impl MigrationPolicy {
    /// Conservative policy: safe metadata and locking operations are allowed;
    /// rewrites, destructive changes, and external rebuilds are denied.
    #[must_use]
    pub const fn conservative() -> Self {
        Self {
            allow_data_rewrite: false,
            allow_destructive: false,
            allow_locking: true,
            allow_external_rebuild: false,
        }
    }

    /// Allows operations which rewrite existing data.
    #[must_use]
    pub const fn with_data_rewrite(mut self) -> Self {
        self.allow_data_rewrite = true;
        self
    }

    /// Allows destructive operations. Data rewrite is also enabled because a
    /// destructive operation is at least as severe.
    #[must_use]
    pub const fn with_destructive(mut self) -> Self {
        self.allow_data_rewrite = true;
        self.allow_destructive = true;
        self
    }

    /// Allows or denies operations which may acquire backend locks.
    #[must_use]
    pub const fn with_locking(mut self, allowed: bool) -> Self {
        self.allow_locking = allowed;
        self
    }

    /// Allows external rebuilds such as index construction.
    #[must_use]
    pub const fn with_external_rebuild(mut self) -> Self {
        self.allow_external_rebuild = true;
        self
    }

    /// Validates all plan risks and required approvals before external work.
    pub fn authorize(
        &self,
        plan: &MigrationPlan,
        approval: Option<&MigrationApproval>,
    ) -> Result<()> {
        if let Some(approval) = approval
            && approval.plan_identity != *plan.identity()
        {
            return Err(Diagnostic::error(
                "MIGRATE-POLICY-001",
                "approval is bound to a different migration plan",
            ));
        }

        for step in plan.steps() {
            let risk = step.risk();
            match risk.data {
                DataSafety::Safe => {}
                DataSafety::RequiresDataRewrite if !self.allow_data_rewrite => {
                    return Err(policy_denial(step.id(), "data rewrite is not allowed"));
                }
                DataSafety::Destructive if !self.allow_destructive => {
                    return Err(policy_denial(
                        step.id(),
                        "destructive changes are not allowed",
                    ));
                }
                DataSafety::RequiresDataRewrite | DataSafety::Destructive => {}
            }
            match risk.operation {
                OperationalImpact::MetadataOnly => {}
                OperationalImpact::Locking if !self.allow_locking => {
                    return Err(policy_denial(
                        step.id(),
                        "locking operations are not allowed",
                    ));
                }
                OperationalImpact::ExternalRebuild if !self.allow_external_rebuild => {
                    return Err(policy_denial(
                        step.id(),
                        "external rebuilds are not allowed",
                    ));
                }
                OperationalImpact::DataRewrite if !self.allow_data_rewrite => {
                    return Err(policy_denial(step.id(), "data rewrite is not allowed"));
                }
                OperationalImpact::Locking
                | OperationalImpact::ExternalRebuild
                | OperationalImpact::DataRewrite => {}
            }
            match risk.automation {
                AutomationLevel::Automatic => {}
                AutomationLevel::RequiresApproval => {
                    let approved = approval
                        .is_some_and(|approval| approval.approved_steps.contains(&step.id()));
                    if !approved {
                        return Err(policy_denial(step.id(), "explicit approval is required"));
                    }
                }
                AutomationLevel::Manual => {
                    return Err(policy_denial(
                        step.id(),
                        "manual steps cannot be executed by the automatic applier",
                    ));
                }
                AutomationLevel::Unsupported => {
                    return Err(policy_denial(
                        step.id(),
                        "the backend does not support this operation",
                    ));
                }
            }
        }
        Ok(())
    }
}

impl Default for MigrationPolicy {
    fn default() -> Self {
        Self::conservative()
    }
}

/// Explicit approval bound to an exact complete plan identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationApproval {
    plan_identity: MigrationPlanIdentity,
    approved_steps: BTreeSet<MigrationStepId>,
    actor: String,
    rationale: String,
}

impl MigrationApproval {
    /// Approves every step in an exact plan.
    pub fn approve_all(
        plan: &MigrationPlan,
        actor: impl Into<String>,
        rationale: impl Into<String>,
    ) -> Result<Self> {
        Self::try_new(
            plan,
            plan.steps().iter().map(|step| step.id()),
            actor,
            rationale,
        )
    }

    /// Approves a selected set of steps in an exact plan.
    pub fn try_new(
        plan: &MigrationPlan,
        steps: impl IntoIterator<Item = MigrationStepId>,
        actor: impl Into<String>,
        rationale: impl Into<String>,
    ) -> Result<Self> {
        let actor = actor.into();
        let rationale = rationale.into();
        if actor.trim().is_empty() || rationale.trim().is_empty() {
            return Err(Diagnostic::error(
                "MIGRATE-POLICY-002",
                "approval actor and rationale must be non-empty",
            ));
        }
        let approved_steps = steps.into_iter().collect::<BTreeSet<_>>();
        let plan_steps = plan
            .steps()
            .iter()
            .map(|step| step.id())
            .collect::<BTreeSet<_>>();
        if !approved_steps.is_subset(&plan_steps) {
            return Err(Diagnostic::error(
                "MIGRATE-POLICY-003",
                "approval contains a step that is not part of its bound plan",
            ));
        }
        Ok(Self {
            plan_identity: plan.identity().clone(),
            approved_steps,
            actor,
            rationale,
        })
    }

    /// Exact plan identity bound to this approval.
    #[must_use]
    pub const fn plan_identity(&self) -> &MigrationPlanIdentity {
        &self.plan_identity
    }

    /// Approved step identifiers.
    #[must_use]
    pub const fn approved_steps(&self) -> &BTreeSet<MigrationStepId> {
        &self.approved_steps
    }

    /// Human or service identity that granted approval.
    #[must_use]
    pub fn actor(&self) -> &str {
        &self.actor
    }

    /// Recorded reason for approval.
    #[must_use]
    pub fn rationale(&self) -> &str {
        &self.rationale
    }
}

fn policy_denial(step: MigrationStepId, reason: &str) -> Diagnostic {
    Diagnostic::error(
        "MIGRATE-POLICY-004",
        format!("migration step {} denied: {reason}", step.get()),
    )
}

#[cfg(test)]
mod tests {
    use crate::catalog::{
        CatalogEntity, CatalogField, CatalogRevision, CatalogScope, CatalogSnapshot,
    };
    use crate::diff::MigrationIntent;
    use crate::plan::{DefaultMigrationPlanner, MigrationPlanner};

    use super::*;

    fn snapshot(revision: &str, include_field: bool) -> CatalogSnapshot {
        let fields = include_field
            .then(|| CatalogField::try_new("secret", "secret", "text", true, false).unwrap())
            .into_iter()
            .collect();
        CatalogSnapshot::try_new(
            CatalogScope::try_new("fake", "test").unwrap(),
            CatalogRevision::try_new(revision).unwrap(),
            vec![CatalogEntity::try_new("user", "users", fields, Vec::new()).unwrap()],
        )
        .unwrap()
    }

    #[test]
    fn destructive_step_needs_policy_opt_in_and_plan_bound_approval() {
        let source = snapshot("one", true);
        let target = snapshot("two", false);
        let plan = DefaultMigrationPlanner::default()
            .plan(&source, &target, &MigrationIntent::default())
            .unwrap();
        let approval =
            MigrationApproval::approve_all(&plan, "operator", "approved change").unwrap();

        let error = MigrationPolicy::default()
            .authorize(&plan, Some(&approval))
            .unwrap_err();
        assert_eq!(error.code(), "MIGRATE-POLICY-004");
        MigrationPolicy::default()
            .with_destructive()
            .authorize(&plan, Some(&approval))
            .unwrap();

        let other = DefaultMigrationPlanner::default()
            .plan(&target, &source, &MigrationIntent::default())
            .unwrap();
        let error = MigrationPolicy::default()
            .with_data_rewrite()
            .authorize(&other, Some(&approval))
            .unwrap_err();
        assert_eq!(error.code(), "MIGRATE-POLICY-001");
    }
}
