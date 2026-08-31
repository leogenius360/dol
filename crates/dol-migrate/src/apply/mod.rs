//! Locked, stale-safe migration application and convergence verification.

use dol_core::diagnostic::{Diagnostic, Result};

use crate::catalog::{CatalogInspector, CatalogLimits, CatalogRevision};
use crate::diff::{DiffLimits, MigrationIntent, diff_catalog};
use crate::plan::{MigrationPlan, MigrationPlanIdentity, MigrationStep, MigrationStepId};
use crate::policy::{MigrationApproval, MigrationPolicy};

/// Bounds on one migration application attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApplyLimits {
    /// Bounds applied to each precondition and convergence inspection.
    pub catalog: CatalogLimits,
    /// Maximum steps that may be sent to a backend.
    pub max_steps: usize,
}

impl Default for ApplyLimits {
    fn default() -> Self {
        Self {
            catalog: CatalogLimits::default(),
            max_steps: 16_384,
        }
    }
}

impl ApplyLimits {
    fn validate(self) -> Result<()> {
        if self.max_steps == 0 {
            return Err(Diagnostic::error(
                "MIGRATE-APPLY-001",
                "apply step limit must be greater than zero",
            ));
        }
        self.catalog.validate()
    }
}

/// How a backend should finish its currently locked migration attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationFinish {
    /// Make the converged catalog visible and release the lock.
    Commit,
    /// Roll back when supported and release the lock.
    Abort,
}

/// Backend primitives required by the checked generic applier.
pub trait CatalogMutator: CatalogInspector {
    /// Acquires the backend migration lock and starts an apply attempt.
    fn begin_migration(&mut self, plan: &MigrationPlan) -> Result<()>;

    /// Applies one ordered typed step while the lock is held.
    fn apply_step(&mut self, step: &MigrationStep) -> Result<()>;

    /// Commits or aborts the attempt and releases the migration lock.
    fn finish_migration(&mut self, finish: MigrationFinish) -> Result<()>;
}

/// Successful application of one plan step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedStep {
    id: MigrationStepId,
    summary: String,
}

impl AppliedStep {
    /// Applied step identifier.
    #[must_use]
    pub const fn id(&self) -> MigrationStepId {
        self.id
    }

    /// Human-readable operation summary.
    #[must_use]
    pub fn summary(&self) -> &str {
        &self.summary
    }
}

/// Proof that a plan was applied and converged under its catalog lock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationReport {
    plan_identity: MigrationPlanIdentity,
    initial_revision: CatalogRevision,
    final_revision: CatalogRevision,
    applied_steps: Vec<AppliedStep>,
    inspections: u32,
}

impl MigrationReport {
    /// Exact plan that was applied.
    #[must_use]
    pub const fn plan_identity(&self) -> &MigrationPlanIdentity {
        &self.plan_identity
    }

    /// Revision observed under the lock before application.
    #[must_use]
    pub const fn initial_revision(&self) -> &CatalogRevision {
        &self.initial_revision
    }

    /// Converged revision observed before commit.
    #[must_use]
    pub const fn final_revision(&self) -> &CatalogRevision {
        &self.final_revision
    }

    /// Steps successfully sent to the backend in order.
    #[must_use]
    pub fn applied_steps(&self) -> &[AppliedStep] {
        &self.applied_steps
    }

    /// Number of locked catalog inspections used as proof.
    #[must_use]
    pub const fn inspections(&self) -> u32 {
        self.inspections
    }
}

/// Applies plans through the locked, policy-checked generic protocol.
pub trait MigrationApplier: CatalogMutator {
    /// Applies a plan, rejects stale state, and proves convergence before commit.
    fn apply_migration(
        &mut self,
        plan: &MigrationPlan,
        policy: &MigrationPolicy,
        approval: Option<&MigrationApproval>,
        limits: ApplyLimits,
    ) -> Result<MigrationReport> {
        apply_checked(self, plan, policy, approval, limits)
    }
}

impl<T> MigrationApplier for T where T: CatalogMutator + ?Sized {}

fn apply_checked<B>(
    backend: &mut B,
    plan: &MigrationPlan,
    policy: &MigrationPolicy,
    approval: Option<&MigrationApproval>,
    limits: ApplyLimits,
) -> Result<MigrationReport>
where
    B: CatalogMutator + ?Sized,
{
    limits.validate()?;
    if plan.steps().len() > limits.max_steps {
        return Err(Diagnostic::error(
            "MIGRATE-APPLY-002",
            format!(
                "plan step count {} exceeds apply limit {}",
                plan.steps().len(),
                limits.max_steps
            ),
        ));
    }
    policy.authorize(plan, approval)?;
    backend.begin_migration(plan)?;

    match apply_while_locked(backend, plan, limits) {
        Ok(report) => {
            backend.finish_migration(MigrationFinish::Commit)?;
            Ok(report)
        }
        Err(primary) => match backend.finish_migration(MigrationFinish::Abort) {
            Ok(()) => Err(primary),
            Err(cleanup) => Err(Diagnostic::error(
                "MIGRATE-APPLY-003",
                format!(
                    "migration failed ({primary}); abort also failed ({cleanup}); backend state is uncertain"
                ),
            )),
        },
    }
}

fn apply_while_locked<B>(
    backend: &mut B,
    plan: &MigrationPlan,
    limits: ApplyLimits,
) -> Result<MigrationReport>
where
    B: CatalogMutator + ?Sized,
{
    let observed = backend.inspect(plan.scope(), limits.catalog)?;
    if observed.revision() != plan.source_revision() {
        return Err(Diagnostic::error(
            "MIGRATE-APPLY-STALE",
            format!(
                "stale migration plan: expected source revision `{}`, observed `{}`",
                plan.source_revision().as_str(),
                observed.revision().as_str()
            ),
        ));
    }
    if !observed.equivalent_catalog(plan.source()) {
        return Err(Diagnostic::error(
            "MIGRATE-APPLY-004",
            "catalog contents do not match the plan source despite an equal revision token",
        ));
    }

    let mut applied_steps = Vec::with_capacity(plan.steps().len());
    for step in plan.steps() {
        backend.apply_step(step).map_err(|error| {
            Diagnostic::error(
                "MIGRATE-APPLY-005",
                format!(
                    "migration step {} failed after {} successful steps: {error}",
                    step.id().get(),
                    applied_steps.len()
                ),
            )
        })?;
        applied_steps.push(AppliedStep {
            id: step.id(),
            summary: step.operation().summary(),
        });
    }

    let final_snapshot = backend.inspect(plan.scope(), limits.catalog)?;
    if final_snapshot.revision() != plan.target_revision() {
        return Err(Diagnostic::error(
            "MIGRATE-APPLY-006",
            format!(
                "migration did not reach target revision `{}`; observed `{}`",
                plan.target_revision().as_str(),
                final_snapshot.revision().as_str()
            ),
        ));
    }
    let convergence = diff_catalog(
        &final_snapshot,
        plan.target(),
        &MigrationIntent::default(),
        DiffLimits {
            max_operations: limits.max_steps,
            max_rename_intents: 1,
        },
    )?;
    if !convergence.is_empty() {
        return Err(Diagnostic::error(
            "MIGRATE-APPLY-007",
            format!(
                "migration did not converge; {} catalog operations remain",
                convergence.operations().len()
            ),
        ));
    }
    Ok(MigrationReport {
        plan_identity: plan.identity().clone(),
        initial_revision: observed.revision().clone(),
        final_revision: final_snapshot.revision().clone(),
        applied_steps,
        inspections: 2,
    })
}

#[cfg(test)]
mod tests {
    use crate::catalog::{
        CatalogEntity, CatalogField, CatalogIndex, CatalogScope, CatalogSnapshot,
    };
    use crate::diff::{MigrationIntent, MigrationOperation, RenameIntent};
    use crate::plan::{DefaultMigrationPlanner, MigrationPlanner};

    use super::*;

    #[derive(Debug)]
    struct FakeBackend {
        current: CatalogSnapshot,
        working: Option<CatalogSnapshot>,
        target_revision: Option<CatalogRevision>,
        expected_steps: usize,
        completed_steps: usize,
        begin_count: usize,
        abort_count: usize,
    }

    impl FakeBackend {
        fn new(current: CatalogSnapshot) -> Self {
            Self {
                current,
                working: None,
                target_revision: None,
                expected_steps: 0,
                completed_steps: 0,
                begin_count: 0,
                abort_count: 0,
            }
        }

        fn visible(&self) -> &CatalogSnapshot {
            self.working.as_ref().unwrap_or(&self.current)
        }

        fn replace_visible_entities(&mut self, entities: Vec<CatalogEntity>) -> Result<()> {
            self.completed_steps += 1;
            let revision = if self.completed_steps == self.expected_steps {
                self.target_revision
                    .clone()
                    .expect("target revision is set while locked")
            } else {
                CatalogRevision::try_new(format!("working-{}", self.completed_steps))?
            };
            self.working = Some(CatalogSnapshot::try_new(
                self.current.scope().clone(),
                revision,
                entities,
            )?);
            Ok(())
        }

        fn replace_entity(
            &mut self,
            key: &str,
            update: impl FnOnce(&CatalogEntity) -> Result<CatalogEntity>,
        ) -> Result<()> {
            let mut entities = self.visible().entities().to_vec();
            let position = entities
                .iter()
                .position(|entity| entity.key() == key)
                .ok_or_else(|| {
                    Diagnostic::error("FAKE-MIGRATE-001", format!("unknown entity `{key}`"))
                })?;
            entities[position] = update(&entities[position])?;
            self.replace_visible_entities(entities)
        }
    }

    impl CatalogInspector for FakeBackend {
        fn inspect(
            &mut self,
            scope: &CatalogScope,
            limits: CatalogLimits,
        ) -> Result<CatalogSnapshot> {
            if self.visible().scope() != scope {
                return Err(Diagnostic::error(
                    "FAKE-MIGRATE-002",
                    "unknown catalog scope",
                ));
            }
            CatalogSnapshot::try_new_with_limits(
                self.visible().scope().clone(),
                self.visible().revision().clone(),
                self.visible().entities().to_vec(),
                limits,
            )
        }
    }

    impl CatalogMutator for FakeBackend {
        fn begin_migration(&mut self, plan: &MigrationPlan) -> Result<()> {
            if self.working.is_some() {
                return Err(Diagnostic::error(
                    "FAKE-MIGRATE-003",
                    "migration lock is already held",
                ));
            }
            self.begin_count += 1;
            self.expected_steps = plan.steps().len();
            self.completed_steps = 0;
            self.target_revision = Some(plan.target_revision().clone());
            self.working = Some(self.current.clone());
            Ok(())
        }

        fn apply_step(&mut self, step: &MigrationStep) -> Result<()> {
            let operation = step.operation();
            match operation {
                MigrationOperation::CreateEntity { entity } => {
                    let mut entities = self.visible().entities().to_vec();
                    entities.push(entity.clone());
                    self.replace_visible_entities(entities)
                }
                MigrationOperation::DropEntity { entity } => {
                    let mut entities = self.visible().entities().to_vec();
                    entities.retain(|candidate| candidate.key() != entity.key());
                    self.replace_visible_entities(entities)
                }
                MigrationOperation::RenameEntity { entity_key, to, .. } => {
                    self.replace_entity(entity_key, |entity| {
                        CatalogEntity::try_new(
                            entity.key(),
                            to,
                            entity.fields().to_vec(),
                            entity.indexes().to_vec(),
                        )
                    })
                }
                MigrationOperation::AddField { entity_key, field } => {
                    self.replace_entity(entity_key, |entity| {
                        let mut fields = entity.fields().to_vec();
                        fields.push(field.clone());
                        CatalogEntity::try_new(
                            entity.key(),
                            entity.name(),
                            fields,
                            entity.indexes().to_vec(),
                        )
                    })
                }
                MigrationOperation::DropField { entity_key, field } => {
                    self.replace_entity(entity_key, |entity| {
                        let mut fields = entity.fields().to_vec();
                        fields.retain(|candidate| candidate.key() != field.key());
                        CatalogEntity::try_new(
                            entity.key(),
                            entity.name(),
                            fields,
                            entity.indexes().to_vec(),
                        )
                    })
                }
                MigrationOperation::RenameField {
                    entity_key,
                    field_key,
                    to,
                    ..
                } => self.replace_entity(entity_key, |entity| {
                    let fields = entity
                        .fields()
                        .iter()
                        .map(|field| {
                            if field.key() == field_key {
                                CatalogField::try_new(
                                    field.key(),
                                    to,
                                    field.data_type(),
                                    field.required(),
                                    field.nullable(),
                                )
                            } else {
                                Ok(field.clone())
                            }
                        })
                        .collect::<Result<Vec<_>>>()?;
                    CatalogEntity::try_new(
                        entity.key(),
                        entity.name(),
                        fields,
                        entity.indexes().to_vec(),
                    )
                }),
                MigrationOperation::AlterField {
                    entity_key, after, ..
                } => self.replace_entity(entity_key, |entity| {
                    let fields = entity
                        .fields()
                        .iter()
                        .map(|field| {
                            if field.key() == after.key() {
                                after.clone()
                            } else {
                                field.clone()
                            }
                        })
                        .collect();
                    CatalogEntity::try_new(
                        entity.key(),
                        entity.name(),
                        fields,
                        entity.indexes().to_vec(),
                    )
                }),
                MigrationOperation::CreateIndex { entity_key, index } => {
                    self.replace_entity(entity_key, |entity| {
                        let mut indexes = entity.indexes().to_vec();
                        indexes.push(index.clone());
                        CatalogEntity::try_new(
                            entity.key(),
                            entity.name(),
                            entity.fields().to_vec(),
                            indexes,
                        )
                    })
                }
                MigrationOperation::DropIndex { entity_key, index } => {
                    self.replace_entity(entity_key, |entity| {
                        let mut indexes = entity.indexes().to_vec();
                        indexes.retain(|candidate| candidate.key() != index.key());
                        CatalogEntity::try_new(
                            entity.key(),
                            entity.name(),
                            entity.fields().to_vec(),
                            indexes,
                        )
                    })
                }
                MigrationOperation::RenameIndex {
                    entity_key,
                    index_key,
                    to,
                    ..
                } => self.replace_entity(entity_key, |entity| {
                    let indexes = entity
                        .indexes()
                        .iter()
                        .map(|index| {
                            if index.key() == index_key {
                                CatalogIndex::try_new(
                                    index.key(),
                                    to,
                                    index.fields().iter().cloned(),
                                    index.unique(),
                                )
                            } else {
                                Ok(index.clone())
                            }
                        })
                        .collect::<Result<Vec<_>>>()?;
                    CatalogEntity::try_new(
                        entity.key(),
                        entity.name(),
                        entity.fields().to_vec(),
                        indexes,
                    )
                }),
            }
        }

        fn finish_migration(&mut self, finish: MigrationFinish) -> Result<()> {
            let working = self.working.take().ok_or_else(|| {
                Diagnostic::error("FAKE-MIGRATE-004", "migration lock is not held")
            })?;
            match finish {
                MigrationFinish::Commit => self.current = working,
                MigrationFinish::Abort => self.abort_count += 1,
            }
            self.target_revision = None;
            Ok(())
        }
    }

    fn source_snapshot() -> CatalogSnapshot {
        CatalogSnapshot::try_new(
            CatalogScope::try_new("fake", "test").unwrap(),
            CatalogRevision::try_new("revision-one").unwrap(),
            vec![
                CatalogEntity::try_new(
                    "user",
                    "users",
                    vec![CatalogField::try_new("id", "id", "u64", true, false).unwrap()],
                    Vec::new(),
                )
                .unwrap(),
            ],
        )
        .unwrap()
    }

    fn target_snapshot() -> CatalogSnapshot {
        CatalogSnapshot::try_new(
            CatalogScope::try_new("fake", "test").unwrap(),
            CatalogRevision::try_new("revision-two").unwrap(),
            vec![
                CatalogEntity::try_new(
                    "user",
                    "people",
                    vec![
                        CatalogField::try_new("id", "id", "u64", true, false).unwrap(),
                        CatalogField::try_new("email", "email", "text", false, true).unwrap(),
                    ],
                    vec![CatalogIndex::try_new("by_email", "by_email", ["email"], true).unwrap()],
                )
                .unwrap(),
            ],
        )
        .unwrap()
    }

    #[test]
    fn inspect_diff_apply_reinspect_converges() {
        let source = source_snapshot();
        let target = target_snapshot();
        let intent =
            MigrationIntent::try_new(vec![RenameIntent::entity("user", "users", "people")])
                .unwrap();
        let planner = DefaultMigrationPlanner::default();
        let plan = planner.plan(&source, &target, &intent).unwrap();
        let approval = MigrationApproval::approve_all(&plan, "operator", "reviewed").unwrap();
        let mut backend = FakeBackend::new(source);

        let report = backend
            .apply_migration(
                &plan,
                &MigrationPolicy::default().with_external_rebuild(),
                Some(&approval),
                ApplyLimits::default(),
            )
            .unwrap();
        assert_eq!(report.applied_steps().len(), 3);
        assert_eq!(report.inspections(), 2);
        assert_eq!(report.final_revision(), target.revision());

        let inspected = backend
            .inspect(target.scope(), CatalogLimits::default())
            .unwrap();
        let converged = planner
            .plan(&inspected, &target, &MigrationIntent::default())
            .unwrap();
        assert!(converged.is_empty());
    }

    #[test]
    fn stale_plan_is_rejected_under_lock_without_mutation() {
        let source = source_snapshot();
        let target = target_snapshot();
        let intent =
            MigrationIntent::try_new(vec![RenameIntent::entity("user", "users", "people")])
                .unwrap();
        let plan = DefaultMigrationPlanner::default()
            .plan(&source, &target, &intent)
            .unwrap();
        let approval = MigrationApproval::approve_all(&plan, "operator", "reviewed").unwrap();
        let mut backend = FakeBackend::new(
            CatalogSnapshot::try_new(
                source.scope().clone(),
                CatalogRevision::try_new("out-of-band").unwrap(),
                source.entities().to_vec(),
            )
            .unwrap(),
        );

        let error = backend
            .apply_migration(
                &plan,
                &MigrationPolicy::default().with_external_rebuild(),
                Some(&approval),
                ApplyLimits::default(),
            )
            .unwrap_err();
        assert_eq!(error.code(), "MIGRATE-APPLY-STALE");
        assert_eq!(backend.begin_count, 1);
        assert_eq!(backend.abort_count, 1);
        assert_eq!(backend.current.revision().as_str(), "out-of-band");
    }

    #[test]
    fn denied_plan_never_acquires_backend_lock() {
        let source = source_snapshot();
        let empty_target = CatalogSnapshot::try_new(
            source.scope().clone(),
            CatalogRevision::try_new("empty").unwrap(),
            Vec::new(),
        )
        .unwrap();
        let plan = DefaultMigrationPlanner::default()
            .plan(&source, &empty_target, &MigrationIntent::default())
            .unwrap();
        let approval = MigrationApproval::approve_all(&plan, "operator", "reviewed").unwrap();
        let mut backend = FakeBackend::new(source);
        let error = backend
            .apply_migration(
                &plan,
                &MigrationPolicy::default(),
                Some(&approval),
                ApplyLimits::default(),
            )
            .unwrap_err();
        assert_eq!(error.code(), "MIGRATE-POLICY-004");
        assert_eq!(backend.begin_count, 0);
    }
}
