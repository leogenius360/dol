# Memory Reference Engine

The memory adapter is DOL's exact executable reference for the semantic language,
pipeline DAG, writes, and eager `DataSet<T>` oracle. The engine-neutral execution
contract lives in the [engine SPI](../design/engine-spi.md).

## Boundary

The protected public conceptual core remains unchanged:

```text
Model
Field<T>
Expr<T>
Pipeline<T>
DataSet<T>
```

`Engine`, `LogicalPlan`, `ExecutionRequest`, `DataStream`, and `LogicalWrite` are execution/support contracts. They do not compete with `Pipeline<T>` as an application data-computation abstraction.

## Foundation

The memory engine is built on these execution guarantees:

1. exact capability claims;
2. remote-only placement and bounded hybrid analysis;
3. typed parameters separated from plan fingerprints;
4. pull-based result streaming;
5. bounded execution;
6. exact source/filter/project/unnest/slice execution;
7. exact writes;
8. serializable multi-write transactions;
9. cross-model referential-integrity validation;
10. reusable conformance helpers.

Unsupported logical operators fail visibly. No operator is approximated to make a test pass.

## Memory registry

`MemoryEngine` owns a semantic model registry and dense `DynRow` tables. Static `DataSet<M>` values and runtime `ModelDef` + `DynRow` values enter the same storage representation.

Before plan execution, the complete registry is validated as one `ModelSet`, then each table is validated for exact row shape, identity, unique constraints, and references.

Static and runtime models therefore receive the same engine semantics.

## Execution

A logical plan is evaluated in validated topological order. Working rows carry two pieces of information internally:

- erased output (`ExecutionRow`);
- model scopes required while evaluating prepared expressions.

Projection closes model scope exactly as the logical plan does. Unnest operates only on projected list values. Filter uses the existing prepared expression evaluator and retains only `Truth::True`.

Parameters are supplied through `Parameters`; they never mutate the logical plan or its fingerprint.
Adapters can inspect the resulting read-only `BoundParameter` values, but untyped callers cannot
forge a canonical binding without first going through a typed `Parameter<T>`.

## Streams and limits

The memory executor materializes reference results eagerly internally, then exposes them through the common pull-stream contract in bounded batches. This is a reference implementation choice, not a requirement that remote engines buffer entire result sets.

It enforces plan/expression limits before execution and per-stage row/byte/timeout limits during
materialization. A zero batch size is invalid. Writes use `WriteLimits`, which independently
bounds affected rows, concurrency, and timeout instead of reusing query-result limits.

## Writes

Typed writes lower to `LogicalWrite`. A non-transactional memory write snapshots only its target
table, applies the candidate mutation, validates the complete registry, and restores the target
table on failure. This avoids cloning unrelated tables while preserving atomic publication.

This preserves the write semantics:

- simultaneous updates;
- three-valued write filters;
- explicit destructive `.all()` acknowledgement;
- canonical write identity;
- rollback on uniqueness/identity/reference failure.

## Transactions

`MemoryTransaction` takes a private snapshot of engine tables. Individual writes must remain model-locally valid, but cross-model references are validated at commit. This permits a transaction to construct mutually consistent final state across multiple writes without requiring every intermediate state to satisfy references.

Rollback simply discards the private snapshot. Commit validates the candidate and publishes it in one engine-state replacement.

## Referential integrity

One isolated `DataSet<M>` cannot enforce a relation across models. The memory
engine performs this check in its multi-model context.

A concrete non-null/non-missing source reference tuple must match one target tuple exactly. Null/missing source tuples are absent references. Model-set validation first confirms that reference widths, types, target fields, and uniqueness contracts are semantically valid.

## Complete exact surface

The memory reference executor supports exact semantics for:

- inner, cross, left, right, and full joins, including null-extended scopes;
- `union`, `union_all`, `intersect`, and `except` with DOL equality/multiplicity semantics;
- deterministic `distinct` and semantic ordering, including explicit Missing/Null placement and IEEE total float ordering;
- global and grouped `count`, `count_present`, exact `sum`, `min`, and `max`;
- `row_number`, `rank`, `dense_rank`, and explicit ROWS-frame exact sums;
- correlated `exists()` / `not_exists()` evaluated at the exact filter position with enclosing-scope bindings.

Materialization checks row, byte, and timeout limits before and during every
stage. Cross joins preflight Cartesian cardinality, and transactions validate
snapshot size before cloning and candidate size before publication.

The full memory suite therefore covers capability honesty, typed parameters,
all currently defined logical operators, pull-stream stability, execution
limits, atomic writes, transactions, and runtime/static referential integrity.
The engine advertises exact support only for semantics exercised by this
reference suite.

Hybrid local-residual execution remains a later optimization capability rather
than a prerequisite for semantic correctness; `RemoteOnly` continues to be the
safe execution policy.
