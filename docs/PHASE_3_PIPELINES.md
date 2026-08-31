# Phase 3 — Pipeline and Logical Plan

Phase 3 turns the completed expression language into DOL's declarative computation-over-data layer.

## Public contract

The protected public mental model is:

```text
Model       describes data
Field<T>    references one typed model value
Expr<T>     symbolically computes one value
Pipeline<T> symbolically computes zero or more values
DataSet<T>  contains concrete/materialized values
```

There is no primary `Query`, `Transform`, `GroupedPipeline`, `JoinPipeline`, or `AggregatePipeline` type.

## Slice 1: pipeline foundation

The first Phase-3 slice established:

- `Pipeline::<M>::from_model()` as a storage-independent semantic model source;
- immutable, cheap-to-clone authoring pipelines;
- `filter(Expr<Truth>)`;
- scalar `select(...)` projection;
- ascending/descending semantic ordering;
- `distinct()` guarded by equality semantics;
- `offset`, `limit`, and combined `slice`;
- bounded pipeline planning via `PipelineLimits`;
- lowering to a topologically ordered `LogicalPlan`;
- prepared expressions retained inside logical nodes;
- BLAKE3 logical-plan fingerprints independent of local `PlanId`/`ExprId`/field slots.

## Slice 2: aliases, multi-source DAGs, joins, and sets

The second slice generalized the authoring graph without changing the public pipeline abstraction:

- `Pipeline::<M>::from_model_as("alias")` introduces an explicit source alias;
- `Field<T>::at("alias")` binds a field to a specific occurrence of a model;
- `join(...)` produces `Pipeline<(L, R)>` using an inner join;
- `cross_join(...)` produces the Cartesian product with no condition;
- `union`, `union_all`, `intersect`, and `except` are ordinary `Pipeline<T>` operations;
- logical plans contain `Join` and `Set` nodes;
- authoring graphs may share subgraphs and are lowered once per unique node;
- duplicate occurrences of the same model require explicit aliases;
- source aliases must be unique within one simultaneous multi-source scope;
- alias spelling is local binding metadata and does not participate in semantic plan fingerprints.

## Slice 3: structural projection, reduction, and outer joins

Slice 3 completes the first coherent relational/dataflow surface while preserving the five-type public core.

### Typed projection

`select(...)` now accepts one symbolic value, an ordered tuple, or a named record:

```rust
let pair: Pipeline<(u64, String)> = Pipeline::<User>::from_model()
    .select((User::id, User::name.trim()));
```

Named records use `#[derive(dol::Projection)]` rather than a separate query/result abstraction:

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

Tuple identity is content-addressed from exact element type fingerprints. Record field declaration order is non-semantic: record projection components are canonicalized by logical field name before plan fingerprinting.

Projection still closes model scopes after producing a new value shape. Durable field access over projected structural values remains a later capability rather than pretending projected records are source models.

### Grouping and aggregation

Reduction remains an operation on `Pipeline<T>`:

```rust
let totals = Pipeline::<Order>::from_model()
    .aggregate((count(), sum(Order::amount)));

let by_customer = Pipeline::<Order>::from_model()
    .aggregate_by(Order::customer_id, (count(), sum(Order::amount)));
```

There is no transient `GroupedPipeline`. Grouping without reduction is already expressible as `select(keys).distinct()`.

Initial aggregate semantics are deliberately narrow and exact:

- `count()` counts input rows and returns `0` for an empty global input;
- `count_present(x)` excludes both Null and Missing values;
- `sum`, `min`, and `max` ignore Missing and return a nullable result when no concrete value exists;
- nullable sources use the explicit `sum_present`, `min_present`, or `max_present` forms, which exclude both Null and Missing;
- `sum` currently requires exact-numeric semantics, so approximate floating-point reduction is rejected until deterministic reduction/NaN semantics are locked;
- `min`/`max` require stable ordering plus key semantics, excluding types whose ordering cannot serve as a stable semantic key;
- grouping keys require equality and keyability recursively;
- a global aggregate produces one result row even for empty input, while `aggregate_by` over empty input produces zero groups.

### Outer joins and nullable scopes

Outer joins are now exposed with exact output nullability:

```rust
let joined: Pipeline<(User, Option<Profile>)> = Pipeline::<User>::from_model()
    .left_join(
        Pipeline::<Profile>::from_model(),
        User::id.eq(Profile::user_id),
    );
```

A non-null field on a nullable outer-join side cannot silently retain `Field<T>` semantics. It must be explicitly lifted:

```rust
let names: Pipeline<Option<String>> = joined.select(Profile::name.nullable());
```

Attempting `joined.select(Profile::name)` is rejected during expression binding. A source field that is already nullable remains directly usable because the unmatched-row state is representable by the same DOL nullable type.

The same rule applies to aliased and runtime fields, and the nullable lift preserves explicit `SemanticBinding<T>` adapters for foreign Rust types.

## Scope rules

A source introduces one model scope. Filter/order/projection/aggregate expressions bind against the scopes currently visible at their input.

For distinct models, ordinary static fields remain enough:

```rust
let pipeline = Pipeline::<User>::from_model().join(
    Pipeline::<Order>::from_model(),
    User::id.eq(Order::user_id),
);
```

A self-join requires aliases because `ModelKey` alone cannot distinguish source occurrences:

```rust
let employees = "employee";
let managers = "manager";

let pipeline = Pipeline::<User>::from_model_as(employees).join(
    Pipeline::<User>::from_model_as(managers),
    User::manager_id
        .at(employees)
        .eq(User::id.at(managers)),
);
```

Aliases are intentionally not semantic identity. Renaming both aliases consistently does not change the canonical pipeline fingerprint. During expression preparation, DOL binds each scoped field to the canonical input-scope position of the logical operation; that bound occurrence identity participates in plan fingerprints.

Inner/cross joins preserve input scope nullability. Left/right/full joins lift the non-preserved source scopes only after the join condition is bound, because the condition is evaluated against the original input rows before null extension.

## Logical plan

The current logical vocabulary is:

```text
Source
Filter
Project
Aggregate
Sort
Distinct
Slice
Join
Set
```

The graph is genuinely multi-input. Pipeline clones share immutable authoring nodes, and lowering compiles each unique node once in dependency order. The public `Pipeline<T>` does not reveal whether its logical root is unary, binary, reduced, or shared.

`PlanOutput` distinguishes model values, scalar/structural values, products, and nullable outer-join outputs. That output vocabulary describes semantics, not a physical row layout.

## Set semantics

Set operations require identical semantic output shapes even when their Rust generic parameter happens to be the same.

- `union`, `intersect`, and `except` require equality semantics for the complete output;
- `union_all` does not require equality because it retains duplicates;
- commutative set operations canonicalize input fingerprints;
- `except` preserves left/right order in its semantic fingerprint.

## Resource safety

`PipelineLimits` bounds the number of unique authoring/logical nodes and carries expression limits used for every retained projection, filter, sort, join, grouping, and aggregate expression. Multi-source authoring graphs are traversed iteratively and lowered in topological dependency order.

## Slice 4: filter-scoped correlation, unnesting, deterministic windows, and rewrites

Slice 4 extends the same `Pipeline<T>` abstraction without adding query-, grouped-, lateral-, or window-pipeline public types.

### Existential truth expressions and ordinary filters

Existence belongs to the expression layer while `filter(...)` remains the one pipeline operation for truth conditions:

```rust
let users_with_orders = Pipeline::<User>::from_model().filter(
    Pipeline::<Order>::from_model()
        .filter(User::id.eq(Order::user_id))
        .exists(),
);
```

`Pipeline::exists()` returns `Expr<Truth>` and therefore composes with ordinary boolean expression operators. `Pipeline::not_exists()` is negation sugar. Correlation is inferred from the field scopes used by nested filter predicates: inner-source fields bind locally, while fields supplied by an enclosing pipeline bind as leading outer scopes at the existential boundary. No separate public `matching(...)` or `correlate(...)` operation exists.

Adjacent filters are canonicalized with logical `AND`, making chained and combined predicate forms semantically identical. Filter chaining is declarative conjunction, not an evaluation-order or short-circuit contract. A correlated nested pipeline cannot be planned independently because its enclosing references remain unresolved; the same pipeline becomes valid when `exists()` is prepared inside an enclosing filter that provides those scopes. Same-model correlation uses explicit source aliases to disambiguate occurrences, but alias spelling does not affect canonical identity.

Prepared nested filters retain their outer-scope count and existential plan dependencies. This keeps correlation visible to validation, fingerprinting, optimizer remapping, and future engines while preserving an ordinary logical `Filter` node rather than introducing a special `SubqueryFilter`. Scalar/cardinality-sensitive subqueries remain a later contract.

### Unnesting

List-valued pipeline outputs can be expanded without introducing a lateral-source abstraction:

```rust
let tags: Pipeline<String> = Pipeline::<User>::from_model()
    .select(User::tags)
    .unnest();
```

`Pipeline<Vec<T>>::unnest()` emits ordered `T` elements. `unnest_present()` is the nullable-list form and maps null/missing lists to zero rows. Because unnest consumes the list-valued output, previous model scopes stay closed.

### Deterministic windows

Window descriptors are supporting authoring values consumed by `Pipeline::window(...)`; the result is still one pipeline, typed as `(input, window_value)`. Initial functions are `rank`, `dense_rank`, `row_number`, and exact `window_sum`. Partition keys require equality/key semantics and order keys require stable ordering/equality/key semantics.

Ranking functions require explicit order and reject frames. `row_number` must prove a total order; initially DOL proves that only for one model source whose identity is still unique in the current rowset and whose complete declared identity appears directly in the order keys. `union` and `union_all` invalidate that uniqueness proof because they can merge colliding model identities. This prevents backend tie ordering from leaking into DOL semantics.

Aggregate windows require an explicit `RowsFrame`; no backend default is accepted. `RowsFrame::all()` is order-independent. Bounded/current-row frames require the same total-order proof. `window_sum` keeps Slice-3 exact-numeric and explicit nullable-source rules.

### Conservative logical optimizer

`LogicalPlan::optimized()` starts the optimizer-equivalence contract with only directly proven rewrites: identity `offset(0)` removal and idempotent repeated-`distinct` removal. The optimized topology may shrink, but the canonical semantic fingerprint is preserved. More powerful rewrites require dedicated semantic-equivalence tests before admission.

## Phase-3 boundary

Phase 3 now has a coherent logical data-computation layer spanning single/multi-source composition, projection, reduction, outer joins, existential correlation, list expansion, deterministic initial windows, and proven logical rewrites. Engine placement/execution remains a later phase: Phase 3 defines what the pipeline means, not where it runs. Scalar subquery cardinality semantics and more aggressive optimizer rules remain explicit future extensions rather than implicit backend behavior.
