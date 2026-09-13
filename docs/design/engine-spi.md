# Engine SPI

The engine SPI separates semantic meaning from physical execution without
introducing another public data-computation abstraction. An engine answers three
independent questions:

1. Can it reproduce the requested DOL semantics exactly?
2. Where may each logical fragment execute under the selected placement policy?
3. How does it physically execute the validated logical form?

## Exact capability contract

`Support` has only three states:

```text
ExactNative
ExactEmulated
Unsupported { reason }
```

An adapter claims support only when the complete DOL semantic contract is
reproducible. A backend operator with a similar name is not sufficient.
Approximate behavior requires an explicitly approximate DOL operation and is
never hidden behind `ExactEmulated`.

The capability matrix treats pipeline nodes, writes, and
transaction/integrity behavior independently. Unsupported claims carry a reason
suitable for placement and explain output.

## Placement

`PlacementPolicy::RemoteOnly` is the default. Every logical node must have exact
engine support or planning fails before execution.

`PlacementPolicy::Hybrid` may mark unsupported nodes as `LocalResidual`, but only
with explicit non-zero row and byte transfer limits and an optional batch limit.
An engine must implement and enforce that transfer boundary before executing a
hybrid placement. Placement never changes logical fingerprints.

## Requests and parameter identity

`ExecutionRequest` carries three independent inputs:

```text
LogicalPlan
Parameters
ExecutionOptions
```

Typed parameter values remain separate from plan identity. Rebinding a parameter
reuses the same logical plan, while the prepared parameter definition still
validates its exact semantic type at execution. Engines receive read-only
`BoundParameter` values; callers populate `Parameters` only through typed
`Parameter<T>` handles.

First-class writes lower through the private `WriteSource` bridge into
`LogicalWrite`, then execute through `WriteRequest`. Physical concerns such as
bulk batching remain outside semantic write identity.

## Pull streams and collection

`DataStream` is synchronous and pull-based:

```text
next_batch() -> Result<Option<ExecutionBatch>>
cancel()
```

The caller controls backpressure. DOL does not force an async runtime into the
base SPI, and engines may perform lazy work between pulls. Every batch carries
the same `PlanOutput`; erased rows use `ExecutionRow`, and collecting them
produces `DataSet<ExecutionRow>` rather than another public result abstraction.

`collect_stream` uses conservative default limits of 100,000 rows and 64 MiB of
logical payload. `collect_stream_with_limits` exposes explicit bounds and
cancels the source on overflow. Differential conformance uses the same bounded
path so a faulty backend cannot make the oracle collect indefinitely.

## Resource boundaries

`ExecutionLimits` bounds logical and expression nodes, rows and logical bytes
materialized by a stage, concurrency, and optional wall-clock time. Hybrid
transfer has a separate budget. Writes use `WriteLimits` for affected rows,
write concurrency, and time rather than borrowing query-result limits.

Recursive structures are depth/node validated before semantic recursion, and
logical DAG compilers traverse iteratively. Every adapter must enforce the limits
for work it claims to execute.

## Cache policy

`CachePolicy` separates cache admission from implementation and requires finite
entry and byte bounds. Logical plans may be shared by semantic fingerprint;
compiled artifacts declare one lifetime:

- `Never`;
- `RequestScoped`;
- `PlanReusable`.

The default policy disables compiled-artifact caching. PostgreSQL and MongoDB
compiled forms currently retain bind values and therefore report
`RequestScoped`. A cache must honor `CacheLifetime`; a plan fingerprint alone
cannot share request-bound values. `PlanReusable` is valid only after templates
are separated from values and the exact adapter/mapping contract participates in
the cache key.

## Transactions

The portable isolation contract starts with `Serializable`.
`EngineTransaction` applies writes to private candidate state and exposes
explicit `commit` and `rollback`. An engine advertising transactions must also
provide atomic writes. Final commit validates the complete candidate state,
allowing temporarily inconsistent intermediate state without accepting an
invalid result.

## Correlation boundary

For existential dependencies, `LogicalExpr::outer_scope_count()` is part of the
binding contract. Leading expression scopes come from the enclosing pipeline
row; remaining scopes belong to the nested stage. This represents correlation
through ordinary nested filters and explicit scope provenance rather than a new
public correlation operation.

## Conformance

`dol-conformance` provides reusable checks for engine identity, capability
invariants, exact remote-only placement, stable stream behavior, write claims,
and transaction/atomicity implications. Concrete adapters add differential
fixtures for their advertised surfaces. The [memory engine](../engines/memory.md)
is the executable reference; [JSONL](../engines/jsonl.md),
[PostgreSQL](../engines/postgresql.md), and [MongoDB](../engines/mongodb.md) must
reject every semantic operation they cannot reproduce exactly.
