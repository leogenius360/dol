# ADR-0006: Multi-source pipelines use local aliases and canonical bound scopes

## Status

Accepted for Phase 3.

## Context

A DOL pipeline must support joins, self-joins, set operations, and shared computation without introducing topology-specific public pipeline types. Static `Field<T>` identifies its owning semantic model and field, but a `ModelKey` cannot distinguish two occurrences of the same model in a self-join.

Human-readable aliases solve authoring ambiguity but should not become durable semantic identity: renaming `employee` to `e` must not invalidate caches or make an equivalent plan appear different.

## Decision

1. `Pipeline<T>` remains the only public deferred-data computation type.
2. Multi-input operations are internal logical DAG nodes.
3. `Pipeline::<M>::from_model_as(alias)` names one source occurrence locally.
4. `Field<T>::at(alias)` selects that occurrence only when disambiguation is required.
5. If a model appears multiple times in one simultaneous scope, every occurrence must be explicitly aliased.
6. Aliases must be unique inside that scope and use bounded ASCII identifiers.
7. Authoring expression fingerprints continue to identify model/field semantics and ignore aliases.
8. Expression preparation records canonical source occurrence positions. Logical-plan expression fingerprints combine normalized expression identity with those bound positions.
9. Source aliases are retained in logical nodes for diagnostics/explain output but are excluded from source and plan semantic fingerprints.
10. Inner and cross joins are implemented before outer joins because they preserve source field nullability. Outer joins require an explicit nullable-scope contract first.

## Consequences

Self-joins are explicit without changing ordinary field ergonomics:

```rust
Pipeline::<User>::from_model_as("employee").join(
    Pipeline::<User>::from_model_as("manager"),
    User::manager_id
        .at("employee")
        .eq(User::id.at("manager")),
)
```

Changing only the aliases does not change the semantic pipeline fingerprint. Engines receive a self-contained logical DAG whose prepared expressions identify input scope positions rather than relying on alias strings.
