# ADR-0003: Portable Function Identity Is Semantic Data

## Status

Accepted for Phase 2.

## Context

DOL expressions need text, temporal, analytical, and eventually extension functions that can be evaluated locally and compiled by different storage engines. Rust closures/function pointers, backend function names, and diagnostic type names cannot provide stable cross-process or cross-backend meaning.

Function behavior can also depend on external semantic data such as Unicode tables. Treating such dependencies as invisible implementation details would allow the same expression fingerprint to mean different things after a toolchain/runtime change.

## Decision

Portable DOL functions are represented by a `FunctionDef` containing:

- a namespaced `FunctionKey`;
- an explicit semantic version;
- a `FunctionDeterminism` classification;
- canonically ordered semantic dependencies.

Function calls participate in canonical expression fingerprints through that definition and their typed argument expressions.

`FunctionDef` never stores a Rust closure or function pointer as semantic identity. Local execution dispatch remains private implementation machinery.

All functions authorable in Phase 2 are deterministic. Type authors may define primitive custom functions through `SemanticFunction`; the trait's local validation/evaluation hooks are execution mechanics and never semantic identity. Custom functions cannot claim DOL's reserved `dol/` namespace. The vocabulary reserves contextual and volatile classifications, but those functions remain un-authorable until an explicit evaluation-context contract exists; Phase 2 therefore cannot hide clock, randomness, session, locale, or environment dependencies inside a custom callback.

## Consequences

- optimizers may constant-fold only deterministic calls with literal inputs;
- engines must match semantic function identity, not merely a similarly named backend function;
- Unicode-sensitive functions can pin their Unicode data version;
- primitive custom functions use `SemanticFunction` without turning their Rust implementation pointer into portable identity;
- function implementation layout remains free to change without changing the public language identity.
