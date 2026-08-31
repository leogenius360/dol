# ADR-0005: Pipeline Is DOL's Public Data-Computation Abstraction

## Status

Accepted for the greenfield reimplementation.

## Context

Earlier DOL designs accumulated overlapping public concepts such as `Query`, `Pipeline`, `Transform`, `Program`, and `Operation`. The overlap made the language harder to explain and encouraged implementation topology to leak into the public API. The historical unary pipeline design also required a separate `BinaryPipeline` when joins and set operations appeared, demonstrating that a linear-stage type was not a sufficient representation for general data computation.

Phase 1/2 established a smaller expression model: a predicate is simply an `Expr<Truth>` used as a condition, and value transformations are ordinary `Expr<T>` composition. The same simplification should apply to computations over collections of data.

## Decision

DOL protects five primary public concepts:

```text
Model
Field<T>
Expr<T>
Pipeline<T>
DataSet<T>
```

Their meanings are:

- `Model`: semantic description of structured data;
- `Field<T>`: symbolic typed reference to one model value;
- `Expr<T>`: symbolic computation producing one value of `T`;
- `Pipeline<T>`: declarative symbolic computation producing zero or more `T` values;
- `DataSet<T>`: concrete/materialized data.

There is no separate primary `Query` or `Transform` Rust type. “Query”, “predicate”, “projection”, and “transformation” remain useful role/domain terminology without requiring parallel public representations.

`Pipeline<T>` is declarative. It lowers to a backend-independent logical DAG. Public method chaining expresses semantic dependency and does not require an engine to execute physical stages in exactly that order.

A model source is semantic rather than physical. `Pipeline::<User>::from_model()` identifies the `User` model shape; storage/engine placement is resolved separately.

Joins, set operations, grouping, aggregation, windows, and future multi-source operations must become logical DAG nodes under the same `Pipeline<T>` abstraction. DOL will not reintroduce `BinaryPipeline`, `JoinPipeline`, or other topology-specific public pipeline types.

## Consequences

- the public mental model is smaller;
- `Query` and `Transform` placeholder modules are removed;
- engine capability terminology uses `pipeline` for declarative data computation;
- logical plan node IDs remain local compiler mechanics and are excluded from semantic fingerprints;
- pipeline fingerprints derive from semantic sources, normalized expressions, operations, and output shape;
- aliases/self-joins require explicit pipeline-local source scopes in a later Phase-3 slice rather than model generics on `Field<T>`;
- `DataSet<T>` remains distinct because it contains actual values rather than describing a computation.
