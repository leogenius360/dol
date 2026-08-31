# Migration

`dol-migrate` is an opt-in, engine-neutral migration control plane. It defines
the safety contract for inspection, diffing, planning, approval, and application;
physical adapters implement the `CatalogInspector` and `CatalogMutator` traits.

## Catalog boundary

`CatalogSnapshot` is a canonical, validated view of entities, fields, and indexes.
Every snapshot carries a backend revision token. `CatalogLimits` bound entities,
fields, indexes, and names before a backend-provided catalog can enter planning.
Snapshots reject duplicate physical names and indexes that reference unknown fields.

The desired and inspected catalogs use stable logical keys separately from mutable
physical names. This allows an operator to state a rename without making name
similarity part of migration semantics.

## Diff and rename intent

`diff_catalog` produces typed `MigrationOperation` values under `DiffLimits`.
Create, drop, alter, index, and rename operations remain distinguishable. Renames
are accepted only through exact `RenameIntent` entries in `MigrationIntent`:

- a matching source key/name and target name must exist;
- an intent cannot be consumed twice;
- unused or ambiguous intent is rejected;
- the planner never guesses a rename from spelling or structural similarity.

## Planning and policy

`DefaultMigrationPlanner` orders operations and assigns a stable step identifier.
Each step is independently classified by:

- `DataSafety`;
- `OperationalImpact`;
- `AutomationLevel`.

`MigrationPolicy` decides which classes may proceed automatically. Risky steps can
require a `MigrationApproval` bound to the exact plan identity, approved step set,
operator, and reason. An approval for a different plan cannot be replayed.

## Apply protocol

`MigrationApplier` implements the guarded protocol over a `CatalogMutator`:

1. validate the plan and policy before acquiring a backend lock;
2. begin the backend migration boundary;
3. inspect again under that boundary;
4. compare the revision and exact source snapshot with the plan precondition;
5. reject a stale plan before the first mutation;
6. apply bounded ordered steps;
7. re-inspect and require an empty follow-up diff;
8. commit on convergence, otherwise abort.

`ApplyLimits` bounds the number of applied steps. Reports contain the plan identity,
applied steps, and final revision without weakening backend errors into success.

## Conformance

`dol-conformance::migration` provides adapter-independent cases for both required
properties:

- inspect → diff → apply → inspect converges to an empty diff;
- a catalog change after planning causes stale rejection without mutation.

The crate intentionally does not issue backend-specific DDL. PostgreSQL, MongoDB,
or another engine must expose its catalog operations through the traits and pass
the same conformance contract before advertising migration support.
