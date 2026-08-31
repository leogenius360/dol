# Phase 4 — Writes and DataSet

Stage E locks DOL's first concrete/materialized execution semantics without turning `Pipeline<T>` into a mutation API.

## Public model

The five primary abstractions remain unchanged:

```text
Model       describes data
Field<T>    references one typed model value
Expr<T>     symbolically computes one value
Pipeline<T> symbolically computes data
DataSet<T>  contains concrete/materialized data
```

Writes remain first-class operations because their safety and result semantics differ from a pipeline:

```rust
User::insert(user)
User::insert_many(users)
User::update().set(User::active, false).filter(User::expired.eq(true))
User::delete().filter(User::expired.eq(true))
User::delete().all()
```

`Update<M, Unscoped>` and `Delete<M, Unscoped>` cannot be applied to a `DataSet<M>`. A destructive operation becomes executable only after `filter(...)` moves it to `Scoped` or `.all()` moves it to `AllAcknowledged`.

## Update semantics

Assignments are simultaneous. Every right-hand expression reads the original row before any assignment in the same update is applied:

```rust
Account::update()
    .set(Account::balance, Account::reserve)
    .set(Account::reserve, Account::balance)
    .filter(Account::id.eq(7))
```

swaps the two values. Assignment authoring order is not semantic and therefore does not affect the canonical write fingerprint. Assigning the same field more than once is rejected instead of defining order-dependent last-write-wins behavior.

Update filters use ordinary DOL three-valued truth. Only `Truth::True` selects a row; `False` and `Unknown` do not. Adjacent write filters canonicalize exactly like pipeline filters, so `.filter(a).filter(b)` and `.filter(a.and(b))` have the same semantic identity without creating a left-deep expression tree.

## Delete semantics

Deletes use the same truth retention rule as pipeline/data-set filtering. `.all()` is a distinct explicit acknowledgement, not shorthand for injecting a literal `true` predicate.

## Inserts

`Insert` and `InsertMany` retain their values as concrete records. Local application validates exact model semantics and the resulting model-local identity/unique constraints before committing the mutation.

`InsertMany::batch_rows(...)` is a physical execution hint. It is validated as non-zero when present but does not participate in semantic fingerprints.

## Atomic local eager application

`DataSet<T>` is the normative eager oracle for Stage E:

```rust
let mut users = DataSet::try_new(initial_users)?;
let outcome = users.apply(
    User::update()
        .set(User::active, false)
        .filter(User::last_seen.lt(cutoff)),
)?;
```

Local writes are atomic with respect to the materialized data set:

- insert rollback removes a candidate row if validation fails;
- bulk insert truncates back to the previous length if validation fails;
- update mutates a cloned candidate and swaps it into the data set only after all expressions, materialization, and constraints succeed;
- delete evaluates every predicate before removing any row.

`WriteOutcome::affected_rows()` means rows inserted, rows selected by an update, or rows deleted. It does not attempt to expose backend-specific notions such as physically changed tuples.

## Record materialization

`RecordView` remains the borrowed read interface. `RecordMut` is an execution capability used for concrete local writes; `#[derive(Model)]` implements it automatically.

Native `DataValue` types provide exact canonical reverse materialization. Application-owned semantic types may override `DataValue::from_datum`. Foreign types use `SemanticBinding<T>::from_datum` on their local binding adapter. The default is an explicit error, so DOL never reconstructs a domain value by guessing from a shared physical representation.

Reverse materialization is deliberately not stored in every `Binding<T>`; write decoding stays at the concrete-record boundary and does not enlarge hot symbolic expression descriptors.

## DataSet validation

`DataSet::new` remains a general materialized collection constructor. `DataSet::try_new` additionally validates a model-shaped data set.

Model validation covers:

- exact model key + definition fingerprint;
- field presence/nullability/type contracts;
- model identity uniqueness;
- declared unique tuples, with null/missing values excluded exactly as defined by `UniqueDef`.

Constraint fingerprints are only indexing buckets. DOL confirms canonical tuple equality inside a fingerprint bucket, so a cryptographic hash collision cannot change identity/unique semantics.

Cross-model referential integrity is intentionally not guessed by one isolated `DataSet<M>`; it requires a multi-model execution/transaction context and belongs with the engine/memory conformance work in Stage F.

## Write identity

Writes have canonical fingerprints over semantic meaning:

- exact model fingerprint;
- operation kind;
- scope acknowledgement/filter;
- normalized bound filter expression;
- canonical field-key order for update assignments;
- normalized bound assignment expressions;
- concrete inserted values in input order.

Local IDs, authoring assignment order, current display names, and physical batch size are excluded.

## Stage F connection

Stage F Slice 1 now lowers these write contracts to engine-facing `LogicalWrite`, executes them atomically in `dol-memory`, binds typed parameters separately from write identity, exposes exact capability analysis, and validates multi-model references at the engine/transaction boundary. Remote database adapters remain later stages.
