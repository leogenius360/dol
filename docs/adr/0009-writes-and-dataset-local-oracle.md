# ADR 0009 — Writes remain first-class; DataSet is the local eager oracle

## Status

Accepted.

## Decision

DOL keeps `Insert`, `InsertMany`, `Update`, and `Delete` as distinct first-class operations instead of adding mutation stages to `Pipeline<T>`.

`DataSet<T>` owns concrete/materialized local semantics. Destructive operations are executable only in `Scoped` or `AllAcknowledged` typestates. Update assignments are simultaneous and canonicalized by stable field key; duplicate assignments are invalid.

Static model records implement the `RecordMut` execution capability through `#[derive(Model)]`. Reverse canonical materialization is supplied by `DataValue::from_datum` or a foreign `SemanticBinding<T>::from_datum`, both of which may explicitly reject unsupported materialization.

## Consequences

- `Pipeline<T>` remains declarative/read-computation vocabulary rather than becoming a command builder.
- `.all()` remains a visible destructive acknowledgement.
- local eager writes provide a normative oracle before backend adapters define physical behavior.
- write fingerprints are independent of assignment authoring order and bulk physical batching hints.
- local update failure is atomic because mutations are committed only after candidate validation.
- foreign/domain values are never reconstructed from representation alone.
