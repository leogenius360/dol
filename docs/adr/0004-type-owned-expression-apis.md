# ADR-0004: Expression APIs Are Owned by Semantic Types

## Status

Accepted for Phase 2.

## Context

`Field<T>` and `Expr<T>` are symbolic values, not concrete instances of `T`. Rust therefore cannot automatically forward inherent methods from `T` to a field or expression. At the same time, making `dol-core` invent and centrally own a parallel method vocabulary for every string, temporal, numeric, domain, scientific, geospatial, or application type would make the language kernel permanently incomplete and increasingly monolithic.

The desired developer model is that `Field<T>` feels like symbolic `T`: familiar operations use the Rust type's vocabulary where the semantic contract is portable, while type authors can add domain methods without modifying DOL core.

## Decision

DOL separates expression composition from type-owned expression APIs.

- `ExprSource` identifies symbolic carriers of one Rust value type. `Field<T>`, `RuntimeField<T>`, `Parameter<T>`, and `Expr<T>` implement it. It exposes `expression(&self)` for Rust-style borrowing APIs and `into_expression(self)` for consuming symbolic operations.
- Type-specific methods are provided through extension traits over `ExprSource<Value = T>`.
- DOL ships such traits for built-in types, and `dol::prelude::*` imports them for ordinary application use.
- User-owned and foreign Rust types can define their own expression extension traits using the same pattern.
- Methods that can be expressed using existing DOL expressions should lower into that algebra directly.
- Semantically primitive methods use a stable deterministic `SemanticFunction`/`FunctionDef`; local validator/evaluator dispatch is private and excluded from semantic identity. Custom functions cannot claim the reserved `dol/` namespace.
- DOL never assumes that every Rust inherent method is portable. Memory-management, pointer, allocation-capacity, platform-width, unchecked, or otherwise runtime-specific APIs are not automatically symbolic operations.
- When an expression method intentionally mirrors a Rust method, its name and semantics should match the Rust/library contract as closely as portable representation permits.

## Consequences

- `Field<String>` and `Expr<String>` share the same symbolic string API without duplicate inherent impls.
- adding a new symbolic carrier automatically gains all type-owned APIs by implementing `ExprSource`;
- custom domain types can publish methods such as `Money::is_zero`, `Money::add_tax`, or `GeoPoint::distance_to` without extending a central DOL enum;
- foreign Rust types remain supported through Phase-1.5 `SemanticBinding<T>` plus an expression extension trait;
- type-owned methods remain normal `Expr<T>` composition, so query planning and engines see semantic expressions rather than type-specific opaque objects;
- primitive custom functions are capability-checked by stable semantic identity rather than by Rust callback identity.
