# Contributing to DOL

DOL uses the repository-owned `xtask` commands as its contributor interface.
The pinned Rust 1.98 toolchain is sufficient for the ordinary workspace; Docker,
nightly Rust, `cargo-fuzz`, and `cargo-deny` are required only for the gates that
explicitly use them.

## Ordinary quality gate

Run before opening a pull request:

```text
cargo xtask check
```

This verifies formatting, all-target/all-feature compilation, strict Clippy,
workspace tests, rustdoc warnings, local Markdown links, the facade feature
matrix, adoption examples, isolated fuzz-target compilation, and repository
metrics. The shell and PowerShell wrappers in `scripts/` are conveniences only;
substantive task logic belongs in `tools/xtask`.

Dependency policy is a separate external-tool gate:

```text
cargo install cargo-deny --locked
cargo xtask security
```

## Public API and examples

`cargo xtask api` compiles the `dol` facade with no features, each optional
feature family independently, and all features together. `cargo xtask adoption`
compiles workspace examples and facade doctests. Keep the public conceptual API
centered on `Model`, `Field<T>`, `Expr<T>`, `Pipeline<T>`, and `DataSet<T>`.

The publishable facade package lives in `dol/`; the repository root is a virtual
Cargo workspace. Use `cargo package -p dol` and `cargo publish -p dol` for an
explicit release target. A local path dependency points to `/path/to/repo/dol`.

## Environment-dependent conformance

Ordinary workspace tests compile but do not claim to execute unavailable live
services. Use the dedicated gates:

```text
cargo xtask postgres-live
cargo xtask postgres-live-external
cargo xtask mongodb-live-external
```

The managed PostgreSQL command requires a running Docker engine. External gates
require `DOL_POSTGRES_TEST_URL` or `DOL_MONGODB_TEST_URL`; private PostgreSQL CAs
may be supplied through `DOL_POSTGRES_TEST_CA`.

## Fuzzing

Stable checks compile every isolated fuzz target. Running libFuzzer requires
nightly Rust and a Unix-like host:

```text
rustup toolchain install nightly --profile minimal
cargo install cargo-fuzz --locked
cargo xtask fuzz
```

Native Windows is unsupported by upstream `cargo-fuzz`; use WSL2/Linux or the
Linux CI job. Corpus, artifact, build, and `.dol/` runtime state remain untracked.

## Performance work

Use [the benchmark contract](docs/performance/benchmarks.md) for exploratory and
canonical commands. Performance closure requires the frozen
[`closure-v1`](docs/performance/closure-v1.md) protocol and qualified-runner
evidence; hosted or local timing is not an acceptance verdict.
