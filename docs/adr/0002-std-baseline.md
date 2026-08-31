# ADR-0002 — `std` Baseline

Status: Accepted

## Decision

DOL 0.1 requires Rust's standard library. `no_std` is not a compatibility
promise and does not constrain the public architecture unless a separately
approved product requirement establishes a concrete embedded/alloc-only use
case.

## Rationale

The primary DOL roadmap includes runtime-defined models, PostgreSQL and MongoDB
engines, filesystem execution, async I/O, analytical operations, migration,
and ML/vector integrations. Optimizing the semantic core around a hypothetical
`no_std` requirement would impose dependency and API constraints before a
product workload justifies them.

The architecture should still avoid gratuitous platform coupling. If an
embedded DOL subset becomes a real requirement, it should receive its own ADR,
capability scope, dependency audit, and conformance target rather than being
assumed to inherit the full DOL language automatically.
