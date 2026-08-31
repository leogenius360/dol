//! Backend-neutral migration conformance scenarios.

use core::fmt::Debug;

use dol_core::diagnostic::{Diagnostic, Result};

/// Adapter implemented by a backend's migration integration tests.
///
/// The adapter owns a configured target catalog. Plans must be produced from
/// the currently inspected catalog to that target.
pub trait MigrationConformanceAdapter {
    /// Backend revision token.
    type Revision: Clone + Debug + Eq;
    /// Backend-specific migration plan.
    type Plan: Clone;

    /// Inspects the current revision.
    fn inspect_revision(&mut self) -> Result<Self::Revision>;

    /// Diffs the current state against the configured target and returns a plan.
    fn plan_to_target(&mut self) -> Result<Self::Plan>;

    /// Source revision bound into a plan.
    fn plan_source_revision(plan: &Self::Plan) -> &Self::Revision;

    /// Number of pending typed steps in a plan.
    fn pending_steps(plan: &Self::Plan) -> usize;

    /// Applies a plan and returns the number of reported applied steps.
    fn apply_plan(&mut self, plan: &Self::Plan) -> Result<usize>;

    /// Performs a catalog change outside the plan, for stale-plan testing.
    fn mutate_out_of_band(&mut self) -> Result<()>;
}

/// Evidence produced by the inspect/diff/apply/converge scenario.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationConformanceReport<R> {
    /// Revision inspected before planning.
    pub initial_revision: R,
    /// Revision inspected after successful application.
    pub final_revision: R,
    /// Number of typed operations applied.
    pub applied_steps: usize,
}

/// Proves inspect, diff, apply, re-inspect, and empty-diff convergence.
pub fn exercise_migration_convergence<A>(
    adapter: &mut A,
) -> Result<MigrationConformanceReport<A::Revision>>
where
    A: MigrationConformanceAdapter,
{
    let initial_revision = adapter.inspect_revision()?;
    let plan = adapter.plan_to_target()?;
    if A::plan_source_revision(&plan) != &initial_revision {
        return Err(Diagnostic::error(
            "CONFORMANCE-MIGRATION-001",
            "planner did not bind the currently inspected source revision",
        ));
    }
    let expected_steps = A::pending_steps(&plan);
    let applied_steps = adapter.apply_plan(&plan)?;
    if applied_steps != expected_steps {
        return Err(Diagnostic::error(
            "CONFORMANCE-MIGRATION-002",
            format!(
                "apply report contains {applied_steps} steps, but plan contains {expected_steps}"
            ),
        ));
    }
    let final_revision = adapter.inspect_revision()?;
    let converged_plan = adapter.plan_to_target()?;
    if A::pending_steps(&converged_plan) != 0 {
        return Err(Diagnostic::error(
            "CONFORMANCE-MIGRATION-003",
            "catalog did not converge to an empty diff after application",
        ));
    }
    Ok(MigrationConformanceReport {
        initial_revision,
        final_revision,
        applied_steps,
    })
}

/// Proves that an out-of-band revision change invalidates a previously built
/// plan without introducing any additional catalog mutation.
pub fn exercise_stale_plan_rejection<A>(adapter: &mut A) -> Result<()>
where
    A: MigrationConformanceAdapter,
{
    let initial = adapter.inspect_revision()?;
    let plan = adapter.plan_to_target()?;
    if A::plan_source_revision(&plan) != &initial {
        return Err(Diagnostic::error(
            "CONFORMANCE-MIGRATION-001",
            "planner did not bind the currently inspected source revision",
        ));
    }
    adapter.mutate_out_of_band()?;
    let changed = adapter.inspect_revision()?;
    if changed == initial {
        return Err(Diagnostic::error(
            "CONFORMANCE-MIGRATION-004",
            "out-of-band test mutation did not change the catalog revision",
        ));
    }
    match adapter.apply_plan(&plan) {
        Ok(_) => {
            return Err(Diagnostic::error(
                "CONFORMANCE-MIGRATION-005",
                "backend accepted a stale migration plan",
            ));
        }
        Err(error) if error.code() != "MIGRATE-APPLY-STALE" => {
            return Err(Diagnostic::error(
                "CONFORMANCE-MIGRATION-006",
                format!(
                    "backend rejected a stale plan with unexpected diagnostic `{}`",
                    error.code()
                ),
            ));
        }
        Err(_) => {}
    }
    let after_rejection = adapter.inspect_revision()?;
    if after_rejection != changed {
        return Err(Diagnostic::error(
            "CONFORMANCE-MIGRATION-007",
            "stale-plan rejection changed the catalog",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone)]
    struct Plan {
        source: u64,
        steps: usize,
    }

    struct FakeAdapter {
        revision: u64,
        target: u64,
    }

    impl MigrationConformanceAdapter for FakeAdapter {
        type Revision = u64;
        type Plan = Plan;

        fn inspect_revision(&mut self) -> Result<Self::Revision> {
            Ok(self.revision)
        }

        fn plan_to_target(&mut self) -> Result<Self::Plan> {
            Ok(Plan {
                source: self.revision,
                steps: usize::from(self.revision != self.target),
            })
        }

        fn plan_source_revision(plan: &Self::Plan) -> &Self::Revision {
            &plan.source
        }

        fn pending_steps(plan: &Self::Plan) -> usize {
            plan.steps
        }

        fn apply_plan(&mut self, plan: &Self::Plan) -> Result<usize> {
            if plan.source != self.revision {
                return Err(Diagnostic::error("MIGRATE-APPLY-STALE", "stale fake plan"));
            }
            self.revision = self.target;
            Ok(plan.steps)
        }

        fn mutate_out_of_band(&mut self) -> Result<()> {
            self.revision += 10;
            Ok(())
        }
    }

    #[test]
    fn generic_convergence_case_proves_empty_follow_up_diff() {
        let mut fake = FakeAdapter {
            revision: 1,
            target: 2,
        };
        let report = exercise_migration_convergence(&mut fake).unwrap();
        assert_eq!(report.initial_revision, 1);
        assert_eq!(report.final_revision, 2);
        assert_eq!(report.applied_steps, 1);
    }

    #[test]
    fn generic_stale_case_proves_rejection_without_mutation() {
        let mut fake = FakeAdapter {
            revision: 1,
            target: 2,
        };
        exercise_stale_plan_rejection(&mut fake).unwrap();
        assert_eq!(fake.revision, 11);
    }
}
