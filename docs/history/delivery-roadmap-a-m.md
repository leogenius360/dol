# Delivery Roadmap

> **Historical snapshot (2026-08-31).** This completed A-M delivery record is
> preserved as observed at revision `772f73a`. For current documentation, use
> the [documentation index](../README.md).

All planned stages are implemented. “Complete” means the repository contains the
specified semantic contract, bounded implementation, tests, and automation. Remote
adapters advertise only the subset they reproduce exactly; unsupported operations
remain explicit placement failures rather than approximate behavior.

| Stage | Scope | Status and delivered boundary |
|---|---|---|
| A | Foundation | **Complete:** Rust 1.98 workspace, facade, crate ownership, xtask, multi-platform CI, formatting/lint/docs/security policy |
| B | Models, types, runtime data | **Complete:** static/runtime metadata equivalence, canonical model/type identity, dense dynamic rows, Missing/Null/Value |
| B.5 | Semantic contract lock | **Complete:** type fingerprints, foreign bindings, canonical values, `std` baseline, adversarial/fuzz coverage |
| C | Expressions and normative semantics | **Complete:** typed expression IR, three-valued truth, normalization, parameters, functions, canonical identity |
| D | Pipeline and logical plan | **Complete:** validated DAG, joins, sets, correlation, unnest, aggregate/window semantics, conservative optimizer |
| E | Writes and `DataSet` | **Complete:** typestate destructive safety, atomic eager reference semantics, constraints |
| F | Engine SPI + memory | **Complete:** exact/node-aware capabilities, bounded pull SPI, complete memory reference executor, writes and bounded serializable transactions |
| G | JSONL | **Complete:** rooted, handle-pinned, bounded incremental source/filter/project/unnest/slice execution |
| H | PostgreSQL | **Complete implementation:** exact mapped source/filter/project/slice compiler, TLS-only bounded runtime, schema preflight, differential live suite, managed PostgreSQL fixture |
| I | MongoDB | **Complete implementation:** exact mapped source/filter/project/slice compiler, Missing/Null BSON contract, bounded official-driver runtime, differential live suite and CI service |
| J | Migration | **Complete:** bounded catalog inspection contract, explicit-intent diff, risk-classified plan, policy/approval, locked apply, convergence and stale-plan rejection |
| K | Wire | **Complete:** versioned canonical envelope, bounded private DTO decode/encode, temporal precision, adversarial tests and fuzz target |
| L | ML/AI | **Complete:** finite dimensioned vectors, deterministic exact search, explicit approximation, capability placement, bounded inference/privacy/cost validation, features and evaluation |
| M | Stabilization | **Complete:** iterative adversarial boundaries, bounded collectors/transactions, cache lifetime policy, release microbenchmarks, feature-matrix API audit, adoption example and strict repository gates |

## Environment-dependent gates

The ordinary workspace suite compiles but ignores tests requiring live databases.
They run through dedicated commands and CI jobs:

```text
cargo xtask postgres-live
cargo xtask postgres-live-external
cargo xtask mongodb-live-external
```

The local PostgreSQL command manages the pinned TLS fixture. The external commands
require `DOL_POSTGRES_TEST_URL` or `DOL_MONGODB_TEST_URL`, respectively. A successful
portable workspace run does not imply that an unavailable live service was tested.
