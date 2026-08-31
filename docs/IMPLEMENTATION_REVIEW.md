# Implementation and Repository Review

Review date: 2026-08-31. Scope: every workspace crate, engine adapter, test family,
workflow, repository task, and roadmap stage.

## Executive result

The workspace compiles on the pinned Rust 1.98 toolchain with all targets/features,
strict clippy, tests, and rustdoc. The implementation follows one semantic source of
truth in `dol-core`; engines either reproduce that contract exactly or reject the
node. Stages A–M are represented by code, tests, documentation, and automation.

The review found no remaining ordinary-build blocker or known semantic test failure.
Live PostgreSQL/MongoDB tests are deliberately environment-dependent: they compile in
the workspace suite but require their dedicated service gates to execute.

## Repository shape

The repository contains 175 Rust source files and 42,759 Rust source lines. Ownership
is coherent:

| Area | Responsibility |
|---|---|
| `dol` | Small application facade and opt-in feature re-exports |
| `dol-core` | Models, types, values, expressions, pipelines, logical plans, writes, fingerprints, local semantics |
| `dol-macros` | Static model/projection derivation into core metadata |
| `dol-engine` | Exact capability SPI, placement, pull streams, limits, cache policy, writes/transactions |
| `dol-memory` | Complete current reference executor and write/transaction oracle |
| `dol-jsonl` | Bounded rooted incremental file adapter |
| `dol-postgres` | Exact mapped PostgreSQL read subset and TLS runtime |
| `dol-mongodb` | Exact mapped MongoDB read subset and bounded driver runtime |
| `dol-migrate` | Engine-neutral guarded catalog migration protocol |
| `dol-wire` | Canonical versioned untrusted-byte boundary |
| `dol-ml` | Vector, placement, inference policy, features, and evaluation |
| `dol-conformance` | Capability, semantic differential, and migration adapter contracts |
| `dol-bench` | Adaptive cross-crate baseline and roadmap performance workloads |
| `xtask`, CI, fuzz | Reproducible repository gates and service-specific validation |

This avoids cycles between application ergonomics and physical backends. PostgreSQL,
MongoDB, JSONL, wire, migration, and ML dependencies do not leak into `dol-core`.

## Findings resolved during implementation

### Build and algorithmic safety

- The original PostgreSQL runtime build failure was repaired by grouping portal pull
  state, reducing the oversized call boundary, clamping driver fetch size, and adding
  boundary tests.
- Type node/depth validation now walks untrusted structures iteratively before the
  recursive semantic pass. A 2,048-level adversarial type is rejected under limits
  before deep semantic recursion.
- PostgreSQL and MongoDB plan compilation now consumes the validated topological DAG
  iteratively instead of recursively descending long stage chains.
- Eager stream and conformance collection gained row/byte bounds and cancellation.

### Exact placement and backend behavior

- `Engine::support_for_node` refines broad capability families against each concrete
  node. PostgreSQL and MongoDB inspect mappings, supported types/operators, total
  truth requirements, correlation, and slice ranges before remote placement.
- The PostgreSQL portal fetch target is capped. When a logical-byte limit is active,
  it requests one row at a time because the synchronous driver materializes a portal
  page before DOL can account for it.
- MongoDB validates every mapped source document before a filter or slice can conceal
  a presence/type violation. Its codec preserves Missing, BSON null, and typed value
  as separate states.
- Both remote compilers parameterize or structurally encode values; neither
  interpolates values into physical query text.

### Local execution and transactions

- Memory operators check materialized row/byte/timeout limits before and during
  output construction. Join cross-products are cardinality-preflighted.
- Transaction snapshots are bounded before cloning, successful writes are counted,
  candidate validation is atomic, and commit performs a final limit/integrity check.
- New tests demonstrate that failed row/byte/write budgets do not partially publish.

### Filesystem and input boundaries

- JSONL bindings pin an open file and compare platform file identity during execution,
  eliminating the earlier validate-path-then-reopen window. Execution uses a fresh
  independent handle only after root, path, pinned, and opened identities agree.
- Wire decoding uses a private DTO, checked lengths, hard byte/node/depth/string/list
  budgets, canonical re-encoding, and exact temporal representations. Random framed
  payload tests and a dedicated fuzz target cover panic resistance.

### Cache and operational safety

- Cache admission is finite and opt-in. Artifact lifetime is explicit; PostgreSQL and
  MongoDB complete compiled results are request-scoped because they retain bind data.
- MongoDB and PostgreSQL runtime configuration debug output redacts credentials.
- Migration plans are revision/snapshot-preconditioned and rechecked under the
  mutation boundary. Approval is bound to an exact plan identity.
- ML external inference validates privacy/data-use policy, cost, shape, deadline, and
  output cardinality before accepting provider results.

## Quality assessment

Strengths:

- semantic identity is canonical and independent of physical mappings;
- Missing/Null/Value remains explicit through core, memory, SQL, BSON, JSONL, and wire;
- capability claims are conservative and explainable;
- parameter values are separate from logical plan identity;
- resource limits exist at definition, expression, plan, execution, stream,
  transaction, migration, wire, vector, inference, feature, and evaluation boundaries;
- DOL-owned crates forbid unsafe Rust;
- tests use memory as the semantic oracle rather than backend-specific expected rules.

Maintainability observation:

- `engines/mongodb/src/compiler.rs` is 1,082 lines and is the only Rust file above the
  repository's 800-line architecture-review threshold. Its source/relational and
  expression lowering are internally separated, but a later no-behavior-change split
  would make ownership easier. This is not a correctness or release blocker.

Deliberate boundaries, not hidden gaps:

- PostgreSQL and MongoDB implement exact source/filter/project/slice reads, not the
  complete memory operator surface. Unsupported nodes fail placement.
- Remote writes/transactions and engine-specific catalog mutators are not advertised.
  The migration crate supplies the generic protocol adapters must implement.
- Compiled artifact caching is policy-only and disabled by default; there is no
  implicit global cache.
- Microbenchmark output is informational. CI does not fail on noisy wall-clock values.

## Verification record

The completion run produced:

| Gate | Result |
|---|---|
| `cargo fmt --all --check` | Pass |
| workspace check, all targets/features | Pass |
| workspace clippy, all targets/features, warnings denied | Pass |
| workspace tests, all features | 234 passed; 0 failed; 8 live-service tests ignored |
| strict workspace rustdoc, all features, no dependencies | Pass |
| facade feature-matrix API audit | Pass |
| adoption example + doctests | Pass |
| isolated fuzz format + strict clippy (`model`, `type`, `expression`, `wire`) | Pass |
| benchmark compilation | Pass |
| `cargo xtask security` (root + isolated fuzz graphs) | advisories, bans, licenses, and sources pass |
| source metrics | 175 Rust files; 42,759 lines; one file above 800 lines |

The optimized adaptive benchmark runners now use nine calibrated samples per
workload. The final core invocation observed 3.65 µs for a 48-level nested-type
validation and 0.88 µs for a representative string-datum fingerprint. The
cross-crate baseline comparison and added engine, wire, migration, vector, and
memory measurements are recorded in [`BENCHMARKS.md`](BENCHMARKS.md).

Docker was unavailable in the review environment, so the seven PostgreSQL and one
MongoDB live tests were not executed locally. Their binaries compiled, PostgreSQL has
the managed TLS xtask/fixture gate, and MongoDB has an external xtask plus pinned CI
service gate. These gates must be green in an environment with the respective service
before claiming a live deployment was exercised.
