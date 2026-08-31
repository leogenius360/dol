# Security Contract

Security follows trust boundaries and bounded resources.

Foundation rules:

- DOL-owned crates forbid unsafe Rust.
- Unscoped update/delete operations are typestate values and are not execution-ready.
- Wire input lowers through bounded private DTO validation rather than direct deserialization into internal execution structures.
- Remote hybrid fallback is opt-in and budgeted.
- Physical values must use backend parameterization; physical identifiers come from validated engine mappings.
- Remote runtime configuration redacts credentials and diagnostics must not expose secret values.
- Migration application requires catalog preconditions and explicit destructive-operation policy.
- External ML placement must obey data-locality/privacy and cost/resource policies.
- Eager collection, engine stages, transactions, migrations, wire decode, and ML operations use finite configurable limits.
- Cache entries require finite admission bounds; request-scoped compiled artifacts cannot be shared across requests.
- JSONL execution pins file identity and revalidates the rooted open handle before reading.
