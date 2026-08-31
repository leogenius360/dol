# ADR-0001 — Semantic Contract Lock

Status: Accepted

## Context

DOL's public API is Rust-native, but portable execution cannot rely on Rust
compiler diagnostics such as `type_name`, `Debug`, or process-local `TypeId`.
The Phase-1 foundation therefore needs an explicit boundary between Rust types
and stable DOL semantics before the expression/compiler layers grow.

## Decision

1. `TypeKey` is the readable semantic lineage identity of a data type. Keys use
   a `namespace/name` form. The `dol/*` namespace is reserved for DOL-defined
   semantics; third-party libraries and applications must use an owned
   namespace such as `org.example/*` or `com.geniustechspace/*`.
2. A `TypeKey` is not the exact type definition. `TypeDef::fingerprint()` is the
   full BLAKE3 fingerprint of the canonical exact definition, including semantic
   version, shape, nullability, properties, and parameters.
3. Conflicting definitions using the same `TypeKey` + semantic version are
   rejected by type-universe validation.
4. Application-owned Rust types normally implement `DataType + DataValue`.
   Foreign Rust types use `SemanticBinding<T>` through a local binding provider,
   avoiding Rust's orphan-rule limitation without newtyping solely for DOL.
5. Canonical DOL values are `Datum`/`Value`; semantic fingerprints and expression
   literals never use `Debug`, display text, pointer identity, or Rust
   `type_name`.
6. Compact IDs, arenas, and interning remain private implementation techniques
   and are introduced only when representative benchmarks justify them.

## Consequences

Rust remains the developer-facing static type system while `TypeDef` is the
portable semantic description after generic Rust type information is erased.
Static models, runtime models, local evaluation, engine compilers, and future
wire formats share one semantic identity/value contract.
