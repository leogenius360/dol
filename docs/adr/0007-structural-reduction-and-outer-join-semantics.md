# ADR 0007 — Structural projection, reduction, and outer-join nullability

## Status

Accepted for Phase 3 Slice 3.

## Decision

DOL keeps `Pipeline<T>` as the only public symbolic computation-over-data abstraction while adding three capabilities:

1. `select(...)` accepts scalar, tuple, and derive-backed named-record projections.
2. `aggregate(...)` and `aggregate_by(...)` represent complete global/grouped reductions without a `GroupedPipeline` intermediate type.
3. left/right/full outer joins represent the non-preserved side as nullable output and nullable source scope.

Named projection records use `#[derive(dol::Projection)]`. They are value types, not models and not query objects.

A non-null `Field<T>` on an outer-join-nullable scope must be explicitly authored as `Field<T>::nullable() -> Expr<Option<T>>`. The same nullable lift is available for aliased and runtime fields and retains foreign `SemanticBinding<T>` semantics. Already-nullable fields remain directly valid.

Initial grouping/aggregate semantics intentionally exclude unresolved approximate behavior. Group keys require equality plus stable keyability. `sum` accepts exact numeric inputs only. `min`/`max` require stable ordering plus keyability. Nullable aggregate sources require explicit `*_present` forms.

## Rationale

Introducing projection-, grouping-, or join-specific pipeline wrapper types would expand DOL's public conceptual core and make fluent composition topology-dependent. Keeping these as logical nodes under `Pipeline<T>` preserves the protected five-abstraction model.

Outer joins cannot honestly expose a non-null `Field<T>` when an unmatched source row can make that value absent. The explicit nullable lift makes the semantic type transition visible in user code, fingerprints, and backend conformance work rather than hiding it in executor behavior.

Aggregate restrictions prefer a smaller exact contract over backend-dependent behavior. Approximate reductions and floating grouping can be added only after their deterministic DOL semantics are specified.

## Consequences

- structural projection output has canonical semantic type identity;
- record declaration order cannot perturb plan identity;
- grouping/reduction remain composable pipeline operations;
- outer-join nullability is represented both in `PlanOutput` and expression scope binding;
- adapters can distinguish exact supported semantics from future approximate extensions;
- projected scopes remain closed until DOL defines a durable field vocabulary for arbitrary structural pipeline values.
