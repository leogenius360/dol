# AI / ML Operations

`dol-ml` treats AI/ML as bounded semantic operations and placement decisions, not
as a model-training framework. Generic aggregates remain in `dol-core`; this crate
owns vector search, features, inference policy, evaluation, and ML capabilities.

## Vector semantics

`Vector` validates a non-empty, bounded dimension and rejects every non-finite
component. `StaticVector<N>` retains the dimension in Rust while using the same
runtime validation. `VectorDefinition` is the portable dimension contract.

The reference distance implementation covers Euclidean, cosine, and inner-product
ordering. Cosine rejects a zero-norm operand instead of manufacturing a score.
`exact_search` validates query/candidate dimensions, bounds candidates and result
count, and uses `VectorId` as the deterministic tie-breaker.

Exact and approximate search are different request variants:

- `ExactSearchRequest` promises complete reference evaluation;
- `ApproximateSearchRequest` requires an explicit `ApproximationTarget` containing
  minimum recall basis points and a probe budget.

An approximate request cannot silently fall back to or masquerade as exact search.

## Capability-aware placement

`MlCapabilities` declares concrete dimensions, metrics, exact-search support, and
approximate targets. `analyze_ml_placement` validates the complete operation shape.
Only exact search is eligible for a local residual implementation; approximate
placement requires a provider that advertises the requested approximation contract.

## Inference boundary

Inference uses a provider trait rather than a global SDK. Requests carry:

- a typed task and bounded input list;
- `PrivacyClass` for each input;
- a `DataUsePolicy` governing external placement;
- item, input-byte, cost, and timeout limits;
- an optional deadline.

`execute_inference` checks provider task support, privacy/data-use rules, quoted
cost, request bounds, deadline, response cardinality, and output type. Diagnostics
and debug representations expose redacted input metadata rather than input content.
Providers cannot return extra/missing outputs or a different task shape as success.

## Features and evaluation

`FeatureName`, `FeatureValue`, and `FeatureSet` enforce bounded names, finite numeric
values, and uniqueness. Reference binary and regression evaluation routines enforce
case limits and deterministic metric formulas, making them useful as conformance
oracles for remote ML systems.
