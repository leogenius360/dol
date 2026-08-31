# ADR 0010 — Engine capabilities are exact claims; memory is the reference executor

## Status

Accepted.

## Decision

DOL engine adapters advertise semantic support as `ExactNative`, `ExactEmulated`, or `Unsupported`. Operator-name similarity is insufficient to claim support, and approximate backend behavior is never hidden behind an exact capability.

`RemoteOnly` is the default placement policy. Hybrid placement requires an explicit bounded transfer budget and may be executed only by adapters that actually enforce the residual boundary.

Typed runtime parameter values are carried separately from logical-plan/write identity. Adapters inspect them through read-only `BoundParameter` values produced only by typed `Parameter<T>` bindings. Query materialization uses `ExecutionLimits`; write affected-row/concurrency/time bounds use the separate `WriteLimits` contract. Execution results use a synchronous pull-stream contract so DOL does not force an async runtime into its base SPI.

The in-memory adapter is the reference executor. It implements operations incrementally: an operation is advertised only after exact local semantics and conformance coverage exist. Slice 1 covers source/filter/project/unnest/slice, Stage-E writes, serializable transactions, and multi-model referential integrity. More complex logical operators remain explicitly unsupported until later Stage-F slices.

## Consequences

- backend adapters cannot silently weaken missing/null, ordering, aggregation, correlation, or write semantics;
- capability analysis can reject unsupported plans before physical execution;
- explain output can distinguish exact engine work from hybrid residual work;
- plan caches may reuse one logical plan across parameter values;
- `DataSet<T>` remains the concrete collection abstraction while `ExecutionRow` is only an erased engine-boundary row shape;
- memory execution becomes a semantic baseline for later PostgreSQL/MongoDB differential tests;
- cross-model constraints live at the engine/transaction boundary rather than being guessed by one isolated `DataSet<M>`.
