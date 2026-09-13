# DOL — Data Operating Language

DOL is a Rust-native semantic language for describing and executing data operations without making a database, file format, or execution engine the center of the API.

This repository is the **greenfield reimplementation**. It follows the new architecture rather than reshaping the historical prototype.

## Architectural shape

```text
DOL language              execution              optional systems
-----------               ---------              ----------------
dol (facade)              dol-engine             dol-migrate
    |                          |                  dol-wire
    +-- dol-core               +-- dol-memory     dol-ml
    +-- dol-macros             +-- dol-jsonl      dol-conformance
                               +-- dol-postgres
                               +-- dol-mongodb
```

The core rule is simple:

> Public Rust ergonomics and the compact form engines eventually execute are different layers.

`dol-core` owns models, fields, expressions, pipelines, analytics, writes, local data, runtime models, logical semantics, plans, and fingerprints. It is intentionally **not** split into `dol-model`, `dol-expr`, `dol-pipeline`, `dol-data`, or `dol-ir` crates.

The repository root is a virtual Cargo workspace. The publishable `dol` facade,
including its source, integration tests, and runnable examples, lives in
[`dol/`](dol/).

## Toolchain

- Rust 1.98.0
- Edition 2024
- Cargo resolver 3
- `#![forbid(unsafe_code)]` in DOL-owned crates
- No Python/Node/Make requirement for repository tasks

## Repository commands

The canonical interface is Rust itself:

```text
cargo xtask check
cargo xtask fmt
cargo xtask lint
cargo xtask test
cargo xtask docs
cargo xtask api
cargo xtask adoption
cargo xtask metrics
cargo xtask security
cargo xtask bench-check
cargo xtask bench
cargo xtask bench-explore -- <criterion arguments>
cargo xtask perf-a-a-smoke -- --artifact-dir <directory>
cargo xtask perf-compare --baseline <revision> --candidate <revision> --profile candidate|release --contract closure-v1 --runner-manifest <manifest> --artifact-dir <directory>
cargo xtask perf-profile --commit <revision> --scenario <scenario-id> --artifact-dir <directory>
cargo xtask fuzz-check
cargo xtask fuzz
cargo xtask ci
```

Dependency-policy checks use the external Rust `cargo-deny` runner. Install it once
for local `cargo xtask security` / `cargo xtask ci` runs:

```text
cargo install cargo-deny --locked
```

GitHub Actions installs `cargo-deny` explicitly in the security job, so repository CI
does not depend on preinstalled runner state.

`cargo xtask fuzz` is an isolated libFuzzer workflow. The normal DOL toolchain
remains stable Rust 1.98; fuzzing additionally requires nightly Rust and the
external `cargo-fuzz` runner on a Unix-like host:

```text
rustup toolchain install nightly --profile minimal
cargo install cargo-fuzz --locked
cargo xtask fuzz
```

Upstream `cargo-fuzz` does not support native Windows. Windows developers should
run fuzzing from WSL2/Linux (preferably with the checkout inside the Linux
filesystem) or rely on the repository's Linux fuzz CI job.

Tiny shell and PowerShell wrappers exist only as conveniences:

```text
./scripts/check.sh
./scripts/check.ps1
```

All substantive logic belongs in `tools/xtask`.

## Current implementation stage

**Roadmap stages A through M are implemented. Remote adapters remain deliberately
capability-bounded: PostgreSQL and MongoDB provide exact mapped read subsets and
reject operations they cannot reproduce. Live database execution is verified by
dedicated service gates, not silently counted as part of an offline workspace run.**

The public conceptual core is intentionally small:

```text
Model       describes data
Field<T>    references one typed model value
Expr<T>     symbolically computes one value
Pipeline<T> symbolically computes data
DataSet<T>  contains concrete/materialized data
```

There is no separate public `Query` or `Transform` abstraction. DOL lowers
storage-independent model pipelines into a validated logical DAG with filtering,
projection, aggregation, ordering, slicing, aliases, joins, set operations,
existential correlation, list unnesting, and deterministic window computation.
Expressions retained by the plan are the same normalized/prepared semantic
expressions used by local evaluation, with canonical source-occurrence binding
and explicit nullable-scope lifting for multi-source plans.

Scalar/cardinality-sensitive subqueries, broader total-order proofs, and aggressive decorrelation/pushdown remain explicit future extensions. The implemented roadmap includes the complete current memory operator/write/transaction oracle, rooted handle-pinned JSONL execution, exact PostgreSQL and MongoDB read adapters, guarded catalog migration, canonical bounded wire encoding, vector/inference semantics, and stabilization policy for limits, caching, benchmarks, API features, adoption examples, security, and fuzzing.

The repository-managed live workflow is:

```bash
cargo xtask postgres-live
cargo xtask postgres-down
cargo xtask mongodb-live-external
```

`postgres-live` requires a running Docker engine, verifies that the managed service is actually healthy instead of trusting stale `.dol/` state, and automatically provisions or restarts the fixture when Docker is available but the service is absent or stopped. `cargo xtask postgres-up` remains available for explicit fixture lifecycle control. The xtask deliberately does not launch Docker Desktop itself; on desktop platforms, start Docker first.

The Docker fixture is pinned by tag and multi-platform image digest to PostgreSQL 18.6, rejects non-TLS host connections, creates a least-privilege `dol_test` role for isolated test schemas, generates an ephemeral local CA/server certificate in a Docker volume, and exports only the CA certificate into ignored `.dol/postgres-test/` state. `postgres-live` automatically trusts that CA. Use `cargo xtask postgres-live-external` with `DOL_POSTGRES_TEST_URL` (and optionally `DOL_POSTGRES_TEST_CA`) to test an external TLS PostgreSQL deployment explicitly.

MongoDB live conformance uses `DOL_MONGODB_TEST_URL` with `cargo xtask mongodb-live-external`; CI runs the same test against a pinned MongoDB service. Both live suites compare canonical results with `dol-memory`.

Start with the [documentation index](docs/README.md),
[architecture](docs/design/architecture.md), and
[normative semantics](docs/design/semantics.md). Contributor workflows and
environment-dependent gates are documented in [CONTRIBUTING.md](CONTRIBUTING.md).

## License

DOL is licensed under the BSD 3-Clause License. See [`LICENSE`](LICENSE).
