# Stabilization Contract

Stage M turns cross-cutting assumptions into executable repository policy.

## Bounded materialization

`collect_stream` now uses conservative defaults (100,000 rows and 64 MiB logical
payload). `collect_stream_with_limits` exposes explicit bounds and cancels the source
on overflow. Differential conformance uses the same mechanism, so a broken backend
cannot make the oracle harness collect indefinitely.

Memory execution performs row/byte/timeout checks before and while materializing
joins, sets, aggregation, windows, unnest, filters, projections, and distinct output.
Cross joins preflight the Cartesian cardinality. Serializable memory transactions
validate snapshot rows/bytes before cloning, bound successful writes, validate each
candidate atomically, and revalidate before commit.

Recursive structural entry points validate depth and node counts iteratively before
semantic recursion. PostgreSQL and MongoDB compile their logical DAG iteratively.

## Cache policy

`dol-engine::CachePolicy` separates admission from implementation and requires
finite entry/byte bounds. Logical plans may be shared by semantic fingerprint.
Compiled artifacts declare one of three classes:

- `Never`;
- `RequestScoped`;
- `PlanReusable`.

The default policy disables compiled-artifact caching. Current complete PostgreSQL
and MongoDB compilation results retain bind values, so both report `RequestScoped`.
A cache must honor the returned `CacheLifetime`; it cannot use a plan fingerprint
alone to share request-bound values. A future adapter may claim `PlanReusable` only
after separating templates from values and including the exact mapping/adapter
contract in its key.

## Benchmarks

`cargo xtask bench-check` compiles the two intentional benchmark targets without
also building every crate's empty libtest harness. `cargo xtask bench` runs them.
The adaptive repository suite preserves the supplied baseline labels and adds
offline PostgreSQL/MongoDB compilation, bounded wire decode, migration planning,
exact vector search, and full memory execution. The focused `dol-core` harness
continues to measure nested-type validation and datum fingerprinting.

The complete workload definitions, inclusion boundaries, initial comparison, and
interpretation policy live in [`BENCHMARKS.md`](BENCHMARKS.md). Measurements are
observations, not portable pass/fail thresholds. Representation changes should be
compared on the same machine/toolchain and must retain semantic and adversarial
tests.

## API and adoption gates

`cargo xtask api` compiles the facade with no features, every optional feature
family independently, and all features together. `cargo xtask docs` treats rustdoc
warnings as errors. `cargo xtask adoption` compiles runnable facade examples and
doctests. These commands are part of `cargo xtask check` and therefore the quality
CI gate.

The repository fuzz command includes model, type, expression, and wire campaigns.
Native Windows remains unsupported by upstream libFuzzer tooling; Linux CI executes
the bounded campaigns. `cargo xtask fuzz-check` formats and strict-clippy-compiles
the isolated fuzz workspace on the stable toolchain and is part of the ordinary
repository check, so stale fuzz code cannot remain hidden until a nightly campaign.
The security task audits both the root and isolated fuzz dependency graphs under the
same source, ban, advisory, and license policy.
