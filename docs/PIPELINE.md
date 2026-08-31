# Pipeline Language

`Pipeline<T>` is DOL's primary declarative data-computation abstraction. It describes a symbolic computation that produces zero or more values of `T`; it does not contain materialized rows and it does not prescribe a physical execution order.

The public mental model is intentionally small:

```text
Model       describes data
Field<T>    references one typed model value
Expr<T>     symbolically computes one value
Pipeline<T> symbolically computes data
DataSet<T>  contains concrete/materialized data
```

There is no separate public `Query`, `Transform`, `GroupedPipeline`, or join-specific pipeline abstraction.

## Current Phase-3 language

The implemented pipeline vocabulary includes:

```text
semantic model source
explicit aliased model source
filter(Expr<Truth>)
select(scalar | tuple | named record)
aggregate(...)
aggregate_by(keys, ...)
order_by / order_by_desc
distinct
offset / limit / slice
inner / left / right / full / cross join
union / union_all / intersect / except
exists / not_exists truth expressions (+ filter-scoped correlation)
unnest / unnest_present
window(rank | dense_rank | row_number | exact sum)
```

A model source is storage-independent:

```rust
let adults = Pipeline::<User>::from_model()
    .filter(User::age.ge(18))
    .order_by(User::name)
    .limit(100);

let plan = adults.logical_plan()?;
```

`Pipeline::<User>::from_model()` means “rows with `User` semantics.” It does not select PostgreSQL, MongoDB, JSONL, memory, or another physical source. Engine binding and placement remain separate concerns.

## Projection

Projection preserves the exact Rust output type while lowering to one structural logical `Project` node:

```rust
let names: Pipeline<String> = Pipeline::<User>::from_model()
    .select(User::name.trim());

let pairs: Pipeline<(u64, String)> = Pipeline::<User>::from_model()
    .select((User::id, User::name.trim()));
```

Named records use `#[derive(dol::Projection)]`:

```rust
#[derive(dol::Projection)]
#[dol(key = "example/user-summary")]
struct UserSummary {
    id: u64,
    name: String,
}

let summaries: Pipeline<UserSummary> = Pipeline::<User>::from_model()
    .select(UserSummary::project((User::id, User::name.trim())));
```

Tuple order is semantic. Named-record declaration order is not: DOL canonicalizes record fields by logical name before fingerprinting.

## Grouping and aggregation

Global and grouped reductions remain ordinary pipeline operations:

```rust
let totals = Pipeline::<Order>::from_model()
    .aggregate((count(), sum(Order::amount)));

let totals_by_user = Pipeline::<Order>::from_model()
    .aggregate_by(Order::user_id, (count(), sum(Order::amount)));
```

There is deliberately no `GroupedPipeline`. A group-only result is `select(keys).distinct()`.

The initial exact aggregate vocabulary is:

- `count()`;
- `count_present(value)`;
- `sum(value)` / `sum_present(nullable_value)`;
- `min(value)` / `min_present(nullable_value)`;
- `max(value)` / `max_present(nullable_value)`.

Grouping keys require equality and stable key semantics. `sum` is limited to exact numeric types; approximate floating-point reduction remains deferred. `min` and `max` require stable ordering plus keyability. Global aggregation always produces one result row; grouped aggregation over empty input produces zero groups.

## Multi-source composition

Different models need no alias when their semantic model keys distinguish them:

```rust
let pipeline = Pipeline::<User>::from_model().join(
    Pipeline::<Order>::from_model(),
    User::id.eq(Order::user_id),
);
```

The output type is `Pipeline<(User, Order)>`.

A self-join uses explicit local source aliases:

```rust
let employee = "employee";
let manager = "manager";

let pipeline = Pipeline::<User>::from_model_as(employee).join(
    Pipeline::<User>::from_model_as(manager),
    User::manager_id
        .at(employee)
        .eq(User::id.at(manager)),
);
```

Aliases are diagnostic/binding names, not durable semantics. The logical fingerprint uses canonical source occurrence position after binding, so consistently renaming aliases does not change pipeline identity.

## Outer joins

Outer joins encode the unmatched side in the Rust output type:

```rust
let profiles: Pipeline<(User, Option<Profile>)> = Pipeline::<User>::from_model()
    .left_join(
        Pipeline::<Profile>::from_model(),
        User::id.eq(Profile::user_id),
    );
```

The same nullability applies to source-field access after the join. If `Profile::name` is non-null in the source model, this is rejected:

```rust
profiles.select(Profile::name)
```

The exact form is explicit:

```rust
let names: Pipeline<Option<String>> = profiles.select(Profile::name.nullable());
```

A field already declared nullable can be used directly. Aliased and runtime fields support the same explicit lift, including fields backed by foreign semantic bindings.

## Set composition

Pipelines with the same Rust output type can request `union`, `union_all`, `intersect`, and `except`. Planning additionally verifies identical semantic output shape. Rust generic equality alone is insufficient because the same Rust representation may carry different DOL semantic bindings.

Duplicate-eliminating set operations require equality semantics recursively across the full output. `union_all` does not.

## Scope behavior

Expressions are validated against semantic model scopes available at the input node. Multi-source joins combine their input scopes. Projection and aggregation produce new values and currently close source-model scopes.

A model-shaped set result preserves compatible model scope, so this remains valid:

```rust
active_users
    .union(invited_users)
    .filter(User::age.ge(18))
```

For an outer join, the join condition binds against the original input scopes. Only the resulting non-preserved scope is then marked nullable for subsequent expressions.

## Existential truth expressions and filter-scoped correlation

Existence is a truth-valued expression; `filter(...)` is the single public operation for row conditions, including conditions that reference an enclosing pipeline:

```rust
let active = Pipeline::<User>::from_model().filter(
    Pipeline::<Order>::from_model()
        .filter(
            Order::user_id
                .eq(User::id)
                .and(Order::status.eq(Status::Pending)),
        )
        .exists(),
);
```

`Pipeline::exists()` produces `Expr<Truth>`, so existential predicates compose with ordinary truth expressions through `and`, `or`, and negation. `not_exists()` is ergonomic sugar for negating `exists()`. The binder classifies each field reference in a nested filter as local or enclosing from source lineage and scope; there is no separate public correlation verb.

Adjacent filters canonicalize to one logical `AND` predicate, so `.filter(a).filter(b)` and `.filter(a.and(b))` have the same logical identity. Filter chaining is declarative conjunction, not an evaluation-order or short-circuit contract. Enclosing references are legal only when the nested pipeline is consumed through an existential boundary that supplies those scopes. Planning the same correlated pipeline independently therefore fails normal scope validation. Same-model correlation uses the existing explicit source-alias rules, while alias spelling remains non-semantic.

The logical plan records how many leading scopes in a prepared nested filter came from an enclosing pipeline. This keeps correlation visible to validation, canonical identity, optimizer remapping, and future engines without adding `matching`, `correlate`, or a special `SubqueryFilter` operation. Scalar/cardinality-sensitive subqueries remain a later contract.

## Unnesting collection outputs

A list-valued projected pipeline can be expanded without introducing a lateral-pipeline abstraction:

```rust
let tags: Pipeline<String> = Pipeline::<User>::from_model()
    .select(User::tags)
    .unnest();
```

`Pipeline<Vec<T>>::unnest()` preserves canonical list order. `Pipeline<Option<Vec<T>>>::unnest_present()` treats null/missing lists as producing zero rows. Because unnest consumes the projected list output, it does not reopen model scopes that projection closed.

## Deterministic window computation

Window descriptors are temporary authoring metadata consumed by `Pipeline::window`; the computation remains a `Pipeline<T>`:

```rust
let ranked = Pipeline::<User>::from_model().window(
    rank()
        .partition_by(User::organization_id)
        .order_by_desc(User::score),
);

let running = Pipeline::<User>::from_model().window(
    window_sum(User::score)
        .order_by(User::id)
        .rows(RowsFrame::to_current()),
);
```

Ranking requires explicit stable ordering. `row_number` additionally requires a provable total order; Slice 4 proves this conservatively for exactly one visible model source whose identity is still unique in the current rowset and when every declared identity field appears directly in the ordering keys. `union` and `union_all` invalidate that uniqueness proof because they can merge rows with colliding model identities. Exact numeric window sums require an explicit `RowsFrame`; zero-distance preceding/following bounds canonicalize to `CurrentRow`, bounded/current-row frames require the same total-order proof, while `RowsFrame::all()` is order-independent. Nullable inputs use `window_sum_present` so null/missing handling is explicit.

## Declarative, not imperative

Method order defines semantic dependency, but it does not force the physical engine to execute one imperative stage after another. `logical_plan()` lowers the authoring pipeline to a backend-independent DAG. `LogicalPlan::optimized()` currently performs only directly proven rewrites: identity `offset(0)` elimination and repeated-`distinct` elimination. Optimized topology may shrink while the original canonical semantic fingerprint is preserved. More aggressive rewrites require dedicated equivalence proofs.

## Logical plan identity

Logical-plan fingerprints are BLAKE3 semantic fingerprints over:

- exact source-model identity;
- normalized and scope-bound prepared-expression identity;
- scalar/tuple/record projection structure;
- grouping and aggregate semantics;
- join/set semantics and outer-nullability shape;
- existential subquery dependency and filter-scoped outer binding;
- unnest element semantics;
- window function, partition, ordering, frame, and result semantics;
- sort direction;
- slice offset/limit;
- output semantic shape.

Human-readable source aliases, local `PlanId`, prepared `ExprId`, field slots, pointers, and physical engine details never participate in semantic plan identity.
