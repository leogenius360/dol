# Engine SPI

Stage F separates semantic meaning from physical execution without adding another public data-computation abstraction.

An engine answers three independent questions:

1. Can this engine reproduce the requested DOL semantics exactly?
2. Where may each logical fragment execute under the current placement policy?
3. How does the assigned engine physically execute the validated logical form?

## Exact capability contract

`Support` has only three states:

```text
ExactNative
ExactEmulated
Unsupported { reason }
```

An adapter must not claim support merely because its backend exposes an operator with the same name. It claims support only when the complete DOL semantic contract is reproducible. Approximate behavior requires a future explicitly approximate DOL operation; it is never hidden behind `ExactEmulated`.

The capability matrix is structural: pipeline nodes, writes, and transaction/integrity behavior are advertised independently. Unsupported claims always carry a reason suitable for placement and explain output.

## Placement

`PlacementPolicy::RemoteOnly` is the default. Every logical node must have exact engine support or planning fails before execution.

`PlacementPolicy::Hybrid` permits unsupported nodes to be marked `LocalResidual`, but only with explicit non-zero row and byte transfer limits (and an optional batch limit). A concrete engine must still implement and enforce the residual-transfer boundary before it can execute such a placement. The Stage-F Slice-1 memory engine can explain hybrid placement but deliberately rejects residual execution.

Placement never changes logical fingerprints.

## Requests and parameter identity

`ExecutionRequest` carries three independent inputs:

```text
LogicalPlan
Parameters
ExecutionOptions
```

Typed parameter values are deliberately separate from plan identity. Rebinding `:minimum` from `10` to `20` reuses the same logical plan; the prepared parameter definition still validates the exact semantic type at evaluation/execution time.

First-class writes lower through the hidden `WriteSource` bridge into `LogicalWrite`, then execute through `WriteRequest`. Physical concerns such as bulk batching remain outside semantic write identity.

## Pull streams

`DataStream` is synchronous and pull-based:

```text
next_batch() -> Result<Option<ExecutionBatch>>
cancel()
```

The caller controls backpressure by deciding when to pull. DOL does not force Tokio, async-std, or another runtime into the base SPI. Engines may perform lazy work between pulls.

`ExecutionBatch` carries the same `PlanOutput` for every batch in a stream. Erased execution rows are represented by `ExecutionRow`; collecting them produces the existing concrete `DataSet<ExecutionRow>` rather than introducing a new result abstraction.

## Resource boundaries

`ExecutionLimits` bounds:

- logical plan nodes;
- retained expression nodes;
- rows materialized by any one execution stage;
- logical bytes materialized by any one execution stage;
- concurrency;
- optional wall-clock execution time.

Writes use a separate `WriteLimits` contract because affected-row limits are not query
materialization limits. It bounds affected rows, write concurrency, and optional wall-clock
write time. Adapters must enforce every limit they claim to execute locally. Hybrid transfer
uses its own explicit transfer budget.

Runtime parameter values remain separate from logical identity. `Parameters` can be populated
only through typed `Parameter<T>` handles, while engine adapters receive a read-only
`BoundParameter` view exposing the already-validated semantic type and canonical `Datum`.

## Transactions

The portable Stage-F transaction isolation contract starts with `Serializable`. `EngineTransaction` applies writes to a private candidate state and exposes explicit `commit` and `rollback`.

If an engine advertises transaction support, atomic writes are mandatory. The memory engine clones its registered tables into a candidate snapshot, applies writes there, validates the complete model set and cross-model references at commit, then publishes the candidate atomically.

This permits temporarily inconsistent intermediate transaction state while still rejecting an invalid final commit.

## Memory reference engine — Slice 1

The memory engine is the first exact execution oracle. Stage F now advertises exact native support for every currently defined logical operator covered by its conformance fixtures:

- model source and ordinary/correlated `filter`;
- scalar/tuple/record projection;
- global/grouped aggregates;
- list unnest;
- deterministic windows with explicit frames;
- semantic sort and distinct;
- offset/limit slicing;
- inner/cross/outer joins;
- set algebra;
- existential/correlated expressions;
- insert / insert-many / simultaneous update / delete;
- serializable transaction commit/rollback;
- model-local identity/unique constraints;
- cross-model referential-integrity validation;
- typed parameter binding;
- pull batching and execution limits.

Hybrid local-residual execution remains intentionally unsupported by the memory adapter. Placement analysis may describe a bounded hybrid plan for other adapters, but the memory reference engine executes only fully engine-placed plans so it never approximates a residual boundary.

## Correlation boundary

For existential dependencies, engines must treat `LogicalExpr::outer_scope_count()` as part of the binding contract. The first `outer_scope_count` expression scopes are supplied by the enclosing pipeline row; remaining scopes belong to the nested pipeline stage itself.

Correlation is therefore represented by ordinary nested `filter(...)` expressions plus explicit scope provenance, not by a separate public correlation or subquery-filter operation.

## Conformance crate

`dol-conformance` provides reusable adapter checks for:

- engine identity and capability invariants;
- exact remote-only placement;
- stable stream output shape and terminal behavior;
- exact write capability claims;
- transaction/atomicity capability implications.

Concrete engine suites then exercise actual semantic fixtures. The complete memory Stage-F suite is the reference implementation for this pattern; PostgreSQL and MongoDB run their advertised read surfaces through canonical differential cases rather than defining backend-specific meaning.


## JSONL incremental engine — Stage G

`dol-jsonl` is the first file-backed adapter. It binds semantic models to files beneath one
canonical filesystem root and advertises exact native support only for source/filter/project/unnest/slice.
The stream opens the file once, freezes the visible byte length, decodes one bounded line at a time,
and applies the supported unary pipeline before producing pull batches.

The adapter has additional hard `JsonlLimits` for source bytes, line bytes, scanned rows, and nested
value depth. Duplicate JSON keys, unknown fields, invalid UTF-8, path traversal, and symlink escape are
errors. Missing properties and explicit JSON null remain distinct DOL states.

Global/materializing and multi-source operators remain `Unsupported` in the JSONL capability matrix,
as do writes and transactions. The adapter does not silently buffer a whole file or delegate to the
memory engine merely to claim a wider capability surface. Differential tests use the memory engine as
the oracle for the streaming-safe subset that JSONL does advertise.


## PostgreSQL compiler, bounded read runtime, and differential gate — Stage H Slice 2

`dol-postgres` keeps physical naming adapter-owned. `PostgresCatalog` maps stable model/field keys to fully quoted schema/table/column identifiers, and optional fields require a separate presence bit so PostgreSQL `NULL` is not overloaded to mean both DOL Missing and DOL Null.

`PostgresEngine::new(...)` remains an offline compiler boundary with an unsupported execution capability matrix. The compiler emits deterministic parameterized SQL plus canonical `SqlBind` values and state/value output columns for the current source/filter/project/slice subset. Unsupported operators or expression families fail compilation rather than being approximated.

`PostgresEngine::with_runtime(...)` explicitly enables the Stage-H read runtime. Runtime configuration requires TLS, and execution validates the live UTF-8/session contract and every referenced mapped table before exposing rows. Canonical driver conversion preserves the two-slot datum state/value representation, including text-to-`numeric` transport for unsigned/wide integer and decimal domains that cannot be represented through the driver's signed integer primitives. Parameterized execution locks the driver contract before execution: DOL derives the exact PostgreSQL parameter types from semantic bind representations, prepares the statement with those types, verifies parameter arity, and binds one PostgreSQL portal inside a read-only transaction. Each `DataStream::next_batch()` fetches a portal page capped at 1,024 physical rows synchronously and fully completes that driver operation before returning the DOL batch. No driver row iterator is suspended across DOL pull commands. This keeps parameter type inference and raw SQL interpolation out of the execution boundary while aligning PostgreSQL backpressure with the engine SPI.

The runtime uses a dedicated synchronous connection worker behind the SPI's pull stream, enforces plan/materialization/timeout limits, and preserves cancellation without imposing an async runtime on DOL. Only source/filter/project/slice are advertised; all writes, transactions, and broader relational operators remain `Unsupported`.

Slice 2B adds an explicit `cargo xtask postgres-live` gate. The default path is repository-managed: the command verifies that the Docker engine is reachable and the Compose service is healthy, then automatically provisions or restarts the digest-pinned PostgreSQL 18.6 fixture when necessary instead of trusting stale local state. `cargo xtask postgres-up` remains the explicit lifecycle command. The fixture forces TLS-only host authentication, generates a private test CA/server certificate inside Docker volumes, and exports only the CA certificate into ignored local state. The xtask does not attempt to launch Docker Desktop itself. External PostgreSQL is intentionally a separate `cargo xtask postgres-live-external` path using `DOL_POSTGRES_TEST_URL` and optionally `DOL_POSTGRES_TEST_CA`, so ambient environment variables cannot redirect the managed gate. `PostgresRuntimeConfig` can add an explicit PEM CA bundle to the platform trust store without disabling normal certificate or hostname verification. `dol-conformance::differential` compares canonical row semantics against `dol-memory`, including NaN/signed-zero normalization, rather than relying on Rust value equality or backend batch boundaries. The live matrix covers physical Missing/Null/Value reconstruction, every currently transported scalar family, parameter states, projection shapes, streaming limits/cancellation/timeout, and incompatible-schema rejection. The normal offline check compiles but does not silently skip-and-pass this proof; Stage H is locked only when the dedicated live command is green.
