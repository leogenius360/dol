# ADR 0008 — Filter-scoped correlation, unnesting, deterministic windows, and conservative rewrites

Status: **Accepted for Phase 3 Slice 4.**

## Context

Slice 3 locked structural projection, exact reduction, and outer-join nullability. The remaining Phase-3 work must add nested dependency, collection expansion, row-set-aware computation, and optimizer freedom without weakening the five-type public core or importing backend-specific SQL behavior.

Four risks dominate this surface:

1. exposing correlation as a second filtering verb duplicates semantics that the binder can infer from field scope;
2. unnesting can accidentally blur symbolic rowsets and materialized collections;
3. window functions become backend-dependent when peer ordering or frame defaults are left implicit;
4. optimizer rewrites can silently change null, missing, ordering, or cardinality semantics.

## Decision

### Existential truth expressions

Phase 3 locks existence as a truth-valued expression and keeps one public predicate verb:

- `Pipeline::filter(predicate)` constrains rows whether the predicate is purely local or references an enclosing pipeline;
- `Pipeline::exists() -> Expr<Truth>` represents existential truth;
- `Pipeline::not_exists()` is ergonomic negation sugar;
- the binder determines local versus enclosing field references from model lineage and source occurrence;
- `exists()` is the boundary at which required enclosing scopes are supplied and validated;
- planning a correlated nested pipeline independently fails ordinary scope validation;
- adjacent filters canonicalize to logical `AND`, so chained and combined forms share semantic identity; filter chaining does not define evaluation order or short-circuit behavior;
- same-model correlation uses the existing source-alias rules;
- aliases remain binding metadata and do not participate in canonical identity;
- other nested stage kinds do not acquire implicit outer-scope capture in this initial contract.

An existential predicate remains inside an ordinary logical `Filter` expression. Nested filter expressions record their number of leading enclosing scopes and reference the nested logical-plan root, allowing canonical fingerprints and optimizer remapping to account for correlation without `matching`, `correlate`, or a special `SubqueryFilter` operation. Existential expressions require pipeline execution context and are therefore not locally evaluated as standalone prepared expressions.

Binding-independent authoring fingerprints hash nested pipeline structure without requiring enclosing scopes. Bound logical fingerprints additionally include outer-scope position and the bound nested-plan fingerprint, keeping self-correlation and source occurrence identity deterministic.

Scalar/cardinality-sensitive subquery expressions are deliberately not inferred from this contract. They require a separate cardinality/error policy beyond existential truth.

### Unnesting

Unnesting operates on a list-valued `Pipeline` output:

```rust
let tags: Pipeline<String> = Pipeline::<User>::from_model()
    .select(User::tags)
    .unnest();
```

`Pipeline<Vec<T>>::unnest()` emits `Pipeline<T>`. `Pipeline<Option<Vec<T>>>::unnest_present()` treats null/missing lists as producing zero element rows. Element order follows the canonical list order. Unnesting closes source scopes because its output is the projected element stream rather than an implicit lateral source model.

### Windows

Window descriptors are temporary authoring metadata consumed by `Pipeline::window`; they are not a sixth primary abstraction. A window stage preserves the input row and appends one typed window value, producing `Pipeline<(T, W)>`.

Initial locked functions are:

- `rank()`;
- `dense_rank()`;
- `row_number()`;
- `window_sum(...)` / `window_sum_present(...)`.

Partition keys require stable equality and key semantics. Ordering keys require stable ordering, equality, and key semantics. Ranking windows require explicit ordering and reject frames.

`row_number()` additionally requires DOL to prove a total order. In the initial implementation that proof is intentionally narrow: exactly one visible model source, source identity must still be unique in the current rowset, and all fields of its declared identity must appear directly in the window ordering. Operators that may merge colliding identities (`union` / `union_all`) deliberately invalidate that proof; subset-style `intersect` / `except` can preserve it. This prevents backend-specific tie order from becoming observable DOL behavior.

Aggregate windows require an explicit `RowsFrame`; there is no backend-default frame. Zero-distance `PRECEDING` / `FOLLOWING` bounds canonicalize to `CURRENT ROW`. Exact sum preserves the Slice-3 numeric/null contract. A full-partition frame is order-independent. Any bounded/current-row frame requires a provable total order for the same reason as `row_number()`.

### Optimizer rewrites

`LogicalPlan::optimized()` performs only rewrites with a direct semantic proof. Slice 4 begins with:

- eliminating `offset(0)` identity slices;
- eliminating repeated `distinct(distinct(x))`.

The optimized plan preserves the original canonical semantic fingerprint. Rewrites are intentionally conservative; lack of a proof means no rewrite. Future pushdown, decorrelation, join reordering, and expression rewrites must be added with targeted equivalence tests rather than enabled by backend convention.

## Consequences

- `Pipeline<T>` remains the only symbolic computation-over-data abstraction.
- Correlation dependencies are visible and resource-bounded.
- Unnesting is typed without inventing a lateral/query abstraction.
- Windows do not inherit SQL engine defaults for peers or frames.
- Optimizer freedom grows from proven equivalences rather than syntactic cleverness.
- Some useful forms are intentionally deferred: scalar subqueries, outer-scope capture in non-filter nested stages, multi-source total-order proof, and aggressive decorrelation/pushdown.
