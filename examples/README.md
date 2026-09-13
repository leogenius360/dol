# DOL Examples

Runnable examples live with the package that owns the demonstrated API. Compile
the complete catalog with `cargo xtask adoption`.

## Facade examples

- [`adoption.rs`](../dol/examples/adoption.rs) derives a model, builds a typed
  filter/projection/slice pipeline, and lowers a logical plan.
- [`optional_systems.rs`](../dol/examples/optional_systems.rs) exercises the
  facade's opt-in subsystems and feature boundaries.
- [`wire_roundtrip.rs`](../dol/examples/wire_roundtrip.rs) demonstrates bounded,
  versioned encoding and decoding.
- [`ml_search.rs`](../dol/examples/ml_search.rs) performs bounded deterministic
  exact vector search; see the [AI/ML guide](../docs/design/ml.md).

Run one from the workspace root, for example:

```text
cargo run --example adoption
```

The virtual workspace selects `dol` as its default member, so existing facade
example commands remain valid after the package move.

## Engine examples

- [`postgres_offline_compile.rs`](../engines/postgres/examples/postgres_offline_compile.rs)
  demonstrates PostgreSQL mapping and compilation without a live database.
- [`mongodb_offline_compile.rs`](../engines/mongodb/examples/mongodb_offline_compile.rs)
  demonstrates MongoDB mapping and compilation without a live database.

Aggregation, windows, runtime-model equivalence, and backend conformance are
executable test suites rather than copy-only snippets. See `dol/tests/`,
`crates/dol-core/tests/`, and each engine's `tests/` directory.
