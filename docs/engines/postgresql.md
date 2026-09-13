# PostgreSQL Engine

The PostgreSQL adapter keeps its physical compiler, bounded runtime, and live
differential proof explicit so DOL never claims database semantics without a
reviewable and testable contract.

## Physical mapping and offline exact compiler

The offline compiler provides:

- adapter-owned `PostgresCatalog`, `TableMapping`, and `ColumnMapping`;
- fully quoted PostgreSQL identifiers with a conservative 63-byte portability cap; logical model/field names are never interpolated as SQL identifiers implicitly;
- optional DOL fields require an explicit boolean presence column, preserving `Missing` separately from SQL `NULL`;
- canonical DOL literals and runtime parameters compile only as PostgreSQL bind placeholders;
- each bound datum uses a stable two-slot `state + typed value` shape, so changing a parameter between `Missing`, `Null`, and `Value` does not rewrite SQL or renumber placeholders;
- deterministic state/value output columns preserve `Missing`, `Null`, and `Value` for the runtime decoder;
- exact single-source `source -> filter -> project -> slice` SQL lowering;
- exact three-valued comparison/filter behavior for the supported scalar subset, including DOL NaN equality/order compensation and `C` collation for deterministic UTF-8 text equality/order;
- a hidden read-only logical-expression adapter view in `dol-core`, so backend compilers never parse display/debug strings or depend on authoring internals.

`PostgresEngine::new(catalog)` remains the offline boundary. It advertises no execution capabilities and opens no network resources.

Current compiler plan operators:

```text
source
filter
project
slice
```

Compiler semantics still deferred:

```text
checked arithmetic / casts
semantic function mappings
unnest
sort / distinct
aggregate / window
join / set
exists / correlation
writes / transactions
```

## Physical Missing / Null contract

A required field uses one value column. A nullable required field maps SQL `NULL` to DOL `Null`.

An optional field uses two columns:

```text
<field>_present boolean NOT NULL
<field>         <physical value type>
```

with the semantic interpretation:

```text
present = false                      -> Missing
present = true, value IS NULL        -> Null       (nullable field only)
present = true, value IS NOT NULL    -> Value
```

For the current live-verification contract, an optional non-null field must use a `NOT NULL` value column as well as a `NOT NULL` presence column. This is deliberately strict: it makes `present = true AND value IS NULL` physically impossible rather than relying on runtime convention.

## Bind and transport policy

DOL values never appear directly in generated SQL. Every expression literal or
runtime parameter that reaches SQL becomes two placeholders: `$n::smallint` for
the DOL datum state (`Missing=0`, `Null=1`, `Value=2`) and a second typed value
slot. The value slot remains present even for `Missing`/`Null`, giving one stable
prepared-statement shape for every runtime state of the same semantic parameter.
Runtime parameters may carry all three states through a typed semantic binding.
Expression literals retain the normative rule that Missing is field/presence
state rather than a standalone literal value.

The runtime makes that parameter contract explicit at the driver boundary. Each encoded bind carries the PostgreSQL type used to serialize it; execution performs a typed prepare, checks the prepared statement's parameter arity, and binds one PostgreSQL portal inside a read-only transaction with the already-typed values. Each DOL pull fetches a positive portal page capped at 1,024 physical rows with `query_portal`, fully completes that driver operation, decodes only the logical rows admitted by the current batch/byte limits, and retains at most the bounded fetched page for the next pull. No synchronous driver `RowIter` remains suspended while the DOL worker waits for another pull command. Wide signed integers, unsigned integers, and decimals continue to bind as `text` because the compiled SQL performs the exact `text -> numeric` conversion. This avoids leaving parameter OID inference to execution while mapping PostgreSQL's native portal paging directly onto DOL's pull/backpressure contract.

Driver-native scalar representations are bound directly where their exact domains match. DOL integer domains wider than signed 64-bit, unsigned integers, and decimal values use canonical decimal text at the driver boundary followed by an explicit PostgreSQL `numeric` cast. The same families are selected back as text and parsed into the canonical DOL value. This avoids narrowing through driver integer types and does not interpolate values into SQL.

PostgreSQL `LIMIT`/`OFFSET` are bounded by its signed bigint execution domain. DOL keeps its storage-independent `u64` API; values above `i64::MAX` are rejected instead of narrowed or delegated to backend overflow behavior.

Native PostgreSQL time/timestamp types remain unsupported because their microsecond precision would silently narrow DOL nanosecond-capable values.

## Bounded read runtime

The runtime builds on the offline compiler with:

- `PostgresEngine::with_runtime(catalog, runtime)` explicitly enables network execution while preserving the offline `new(...)` constructor;
- `PostgresRuntimeConfig` accepts PostgreSQL connection configuration only when `sslmode=require` is explicit;
- the native TLS connector uses the platform certificate/hostname verifier; code does not disable certificate or hostname validation;
- connection establishment is bounded by an explicit/default connect timeout and the execution timeout when it is tighter;
- every execution verifies the live PostgreSQL session before reading:
  - `server_encoding = UTF8`;
  - server identifier capacity is at least the adapter's validated 63-byte contract;
- every referenced mapped table is inspected through `pg_catalog` before execution and must match the registered physical contract:
  - mapped table exists as a base or partitioned table;
  - value columns use the exact supported PostgreSQL base type;
  - required/non-null DOL values have a `NOT NULL` physical guarantee;
  - optional presence columns are base `boolean NOT NULL`;
  - PostgreSQL domains are not silently treated as equivalent base types;
- state/value result columns are normalized into stable driver-decodable transport types;
- result decoding reconstructs `Missing`, `Null`, and `Value` and revalidates every decoded datum against its DOL semantic type;
- scalar, tuple, record, and model outputs from the currently supported projection surface are reconstructed explicitly;
- execution uses one dedicated worker connection per stream and a synchronous pull protocol with bounded batch size;
- materialized-row, materialized-byte, plan-size, expression-size, concurrency, and timeout limits remain enforced through the engine SPI;
- cancellation prevents further caller-visible pulls and queues worker shutdown without requiring an async runtime;
- when DOL supplies an execution timeout, PostgreSQL `statement_timeout` is only tightened (never relaxed); an existing stricter server/session timeout is preserved, and no timeout is disabled when DOL leaves the limit unset;
- public `execute(...)` and `explain(...)` mirror the memory and JSONL engine ergonomics.

### Runtime capability claims

Only the read surface already lowered exactly by the compiler is advertised:

```text
source   = ExactNative
filter   = ExactEmulated
project  = ExactEmulated
slice    = ExactNative
```

All other pipeline operators remain unsupported. All write and transaction capabilities remain unsupported. A compiler rejection still wins for an unsupported expression inside an otherwise supported operator; the adapter never substitutes approximate SQL.

## Live differential gate

The live harness deliberately adds no capability breadth: its purpose is to
prove the advertised read runtime against a real PostgreSQL server and the
`dol-memory` semantic oracle.

The reusable `dol-conformance::differential` layer compares canonical typed datum fingerprints rather than Rust `PartialEq`, so NaN payloads, signed zero, decimal normalization, structured values, and model-field semantics are compared according to DOL contracts. It supports both ordered stream comparison and unordered multiset comparison for plans that establish no result ordering. Backend batch boundaries are non-semantic.

The ignored `dol-postgres` live suite creates a fresh isolated schema for each case and covers:

1. `Missing` / `Null` / `Value` reconstruction for required, optional, nullable, and optional-nullable model fields;
2. bool/truth, every signed and unsigned integer width, decimal, float edge cases (`NaN`, infinities, signed zero), UTF-8 text comparison, bytes, UUID, char, and date values currently accepted by the compiler;
3. runtime parameters in `Missing`, `Null`, and `Value` states with one stable SQL/placeholder shape; expression literals continue to obey the rule that Missing is not a standalone literal;
4. model, scalar, tuple, and named-record projection decoding plus singleton slice behavior;
5. pull batching, cancellation, row/byte materialization bounds, and a real PostgreSQL `statement_timeout` failure induced by a conflicting table lock;
6. deliberately incompatible live schemas proving validation fails before any result stream is exposed.

The live gate is explicit rather than silently skipped. The default repository-managed workflow is:

```text
cargo xtask postgres-live
cargo xtask postgres-down
```

`postgres-live` first proves the Docker engine is reachable, inspects the managed Compose service, and waits for a real PostgreSQL health probe. If Docker is running but the fixture is absent or stopped, it automatically provisions or restarts it while preserving the previously selected managed port when possible. Stale `.dol/postgres-test/` files are therefore never sufficient to launch the live tests. The xtask does not attempt to launch Docker Desktop itself; the Docker engine must already be running. `postgres-up` remains available for explicit setup.

The managed setup uses `infra/postgres-test/compose.yaml` to build a digest-pinned PostgreSQL 18.6 fixture. The server rejects non-TLS host connections, uses SCRAM authentication, and gives the `dol_test` principal only the database `CONNECT`/`CREATE` rights needed to create isolated per-test schemas. A local CA and localhost server certificate are generated inside Docker-managed TLS storage; the CA private key remains there. Only `ca.crt` and the selected host port are copied into ignored `.dol/postgres-test/` state. `postgres-down` removes the database/TLS volumes and local state.

For an external database, use `cargo xtask postgres-live-external` with `DOL_POSTGRES_TEST_URL=postgresql://.../?sslmode=require`; if it uses a private CA, also set `DOL_POSTGRES_TEST_CA` to a PEM CA file. `PostgresRuntimeConfig::with_trusted_ca_pem` / `with_trusted_ca_file` add that CA while retaining normal platform roots and hostname verification. On PowerShell, set the same variables through `$env:...`. Connection URLs are never printed by the xtask. The normal `cargo xtask check` compiles the live test target under Clippy/tests but does not execute ignored network tests.

The live boundary is verified only when `cargo xtask postgres-live` passes
against a real PostgreSQL instance. Capability breadth must not expand into
checked arithmetic/functions or additional relational operators without the
same proof.

## Future extensions

Writes and transactions are intentionally separate from the read-runtime gate. `insert`, bulk insert, update, delete, explicit transactions, atomic write behavior, and serializable transaction semantics must each be implemented and differentially proven before their corresponding capabilities are advertised.
