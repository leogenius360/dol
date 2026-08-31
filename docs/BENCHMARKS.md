# Benchmark Contract

The repository owns two dependency-free optimized benchmark targets:

- `dol-bench/roadmap` measures the supplied baseline labels plus representative
  roadmap paths across core, engines, migrations, wire, and ML;
- `dol-core/semantic_hot_paths` probes recursive type validation and canonical
  datum fingerprinting.

Run both with `cargo xtask bench`; compile them without executing with
`cargo xtask bench-check`. Both runners calibrate each workload, collect nine
approximately 150 ms samples, and report the median throughput, median latency,
observed sample range, and calibrated iterations. They deliberately have no
timing threshold: machine load, power policy, compiler changes, and CPU frequency
make performance results unsuitable as correctness gates.

## Baseline comparison contract

The initial figures supplied for 2026-08-23 did not include their fixture source,
and no original matching implementation was present before this harness was added.
The suite therefore preserves a concrete reconstructed contract for the five
labels. Its percentage deltas are directional rather than strict apples-to-apples
regressions until an original fixture can prove equivalence.

| Label | Preserved workload |
| --- | --- |
| Expression construction | Build `active == true AND balance >= 10 OR nickname is missing` over the four-field `BenchAccount` model. |
| Expression evaluation | Evaluate the prepared expression above against one fixed `BenchAccount` and a reused `EvalContext`. |
| Expression/pipeline clone plus fingerprints | Clone the expression and a source/filter/offset/limit pipeline, then compute both semantic fingerprints. |
| PostgreSQL capability analysis and planning | Lower that typed pipeline, run concrete PostgreSQL `RemoteOnly` capability placement, and construct its explain plan using an offline TLS-required runtime configuration. No connection is opened. |
| Runtime model definition | Build and freeze the four-field account model with identity and optional-field uniqueness constraints. |

The two representation measurements use `size_of` on `Expr<Truth>` and
`Pipeline<BenchAccount>` and therefore are directly comparable to byte-size
baselines.

## Additional roadmap coverage

The roadmap runner also measures:

- PostgreSQL exact offline compilation of the fixed source/filter/offset/limit
  logical plan;
- MongoDB exact offline compilation of that same plan;
- bounded decode of an encoded `Vec<Option<String>>` `TypeDef` frame;
- migration diffing and risk-classified planning from one four-field entity to a
  five-field entity with a new unique index (two changes);
- exhaustive Euclidean vector search over 128 owned 64-dimensional candidates,
  returning the top 10; candidate cloning is included because the exact-search
  API consumes candidates;
- full memory-engine lowering, execution, and bounded stream collection for the
  fixed pipeline over 128 input records.

The core harness separately measures validation of a 48-level nested list type and
fingerprinting a 144-byte string datum. Keep workload names, shapes, and inclusion
boundaries stable when comparing runs; introduce a new label when changing them.

## 2026-08-31 local runs

Three invocations of the preserved contract on the development Windows host, Rust
1.98.0, and Cargo's optimized `bench` profile produced the following band between
their independently calculated nine-sample medians:

| Measurement | Current median band | Supplied baseline | Directional delta band |
| --- | ---: | ---: | ---: |
| Expression construction | 361,150–404,555 ops/s | 310,598 ops/s | +16.3% to +30.3% |
| Expression evaluation | 964,528–1,117,340 ops/s | 7,950,175 ops/s | -87.9% to -85.9% |
| Expression/pipeline clone plus fingerprints | 17,326–19,835 pairs/s | 49,481 pairs/s | -65.0% to -59.9% |
| PostgreSQL capability analysis and planning | 26,554–31,001 plans/s | 68,660 plans/s | -61.3% to -54.8% |
| Runtime model definition | 88,740–94,630 models/s | 97,468 models/s | -9.0% to -2.9% |
| `Expr` handle stack size | 24 bytes | 32 bytes | -8 bytes |
| `Pipeline` handle stack size | 8 bytes | 32 bytes | -24 bytes |

Additional median bands were 8,863–9,398 PostgreSQL compilations/s, 922–1,051
MongoDB compilations/s, 397,242–529,936 wire decodes/s, 78,765–84,170 migration
plans/s, 11,570–22,758 exact vector searches/s, and 2,280–2,460 complete memory
executions/s. The wide vector-search band is itself evidence that scheduler noise
must be ruled out before treating a timing change as an implementation regression.

The final invocation of the adaptive core harness measured 273,644 nested-type
validations/s (3,654.4 ns/op) and 1,130,130 datum fingerprints/s (884.9 ns/op).

These observations do not justify an optimization rewrite by themselves. Repeat
the suite on the same idle host/toolchain, retain the full sample range, profile a
confirmed regression, and preserve semantic tests before changing implementation.
