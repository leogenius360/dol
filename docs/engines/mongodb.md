# MongoDB Engine

The MongoDB engine is an exact, bounded read adapter over the official
synchronous Rust driver. It does not treat MongoDB behavior as DOL semantics:
the adapter advertises only operations for which it supplies an exact lowering.

## Mapping

`MongodbCatalog` binds an exact `ModelDef` fingerprint to a physical database and
collection. `CollectionMapping` must cover every model field exactly once, rejects
unknown keys, reserved `__dol_*` output names, NULs, empty names, and stale model
definitions. Physical names never enter model or plan identity.

The supported scalar transport is deliberately narrow: boolean/truth, integers up
to 64 bits, unsigned integers up to 64 bits using exact decimal transport, `f64`,
character, string, bytes, and UUID. Types without a proven lossless BSON mapping are
rejected during mapping or compilation.

## Missing, null, and value

MongoDB missing and BSON null are not interchangeable. Source and projected values
are represented internally as a pair:

```text
state = 0 (Missing), 1 (Null), or 2 (Value)
value = typed BSON payload when state == 2
```

Every execution first runs a bounded-result validation aggregation over the whole
mapped collection. This prevents a filter or slice from hiding a malformed source
document. Only after preflight succeeds does the requested aggregation produce rows.

## Exact compiler surface

The offline `MongodbCompiler` lowers a topologically validated single-source plan:

- source;
- truth filter;
- scalar, tuple, or named-record projection;
- offset/limit slice representable by MongoDB `i64` bounds.

Supported expressions include fields, typed literals/parameters, presence tests,
nullable lifting, comparisons with DOL unknown semantics, truth operations when
their inputs are total values, null-safe equality, coalesce, membership, and
conditional expressions. Arithmetic, functions, correlation, joins, aggregation,
windows, sort, distinct, set operations, and writes are rejected explicitly.

Node-aware placement refines the coarse capability matrix against the concrete
mapping, types, expression graph, total-truth requirements, and slice range before
execution. Compiler artifacts embed typed parameter BSON and are therefore marked
`RequestScoped` for cache policy.

## Runtime

`MongodbEngine::with_runtime` is the explicit network-enabled constructor. Runtime
configuration redacts URI credentials in debug output and supplies connection and
server-selection timeouts. Execution validates plan limits, requires fully remote
exact placement, performs collection preflight, and opens an aggregation cursor.

The pull stream provides natural backpressure, terminal cancellation, bounded
driver batch size, logical row/byte accounting, and timeout checks. A configured
logical-byte bound forces conservative driver paging so an adapter does not fetch an
unbounded batch before DOL can account for it.

## Verification

Offline tests cover mapping staleness, Missing/Null compilation, deterministic
pipelines, unsupported semantics, slice ranges, redaction, cache classification,
and plan-aware capability rejection. The ignored live test loads the same semantic
rows into MongoDB and `dol-memory`, then compares canonical unordered results for
source, filter, projection, slice, parameters, and presence states.

Run a live fixture with:

```text
DOL_MONGODB_TEST_URL=mongodb://localhost:27017/?directConnection=true \
  cargo xtask mongodb-live-external
```

GitHub Actions runs this gate against the pinned `mongo:8.0.29-noble` service.
