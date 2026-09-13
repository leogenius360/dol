# Benchmark Contract

DOL keeps hosted timing informational and correctness gates deterministic.
Scheduler load, CPU boost state, power policy, compiler changes, and background
services can move a hosted run materially. Only paired `closure-v1` evidence
from a qualified dedicated runner can enforce a performance verdict. The
measurement and closure rules are in [the closure-v1 contract](closure-v1.md);
the preceding optimization record remains in the historical
[2026-09-01 milestone report](../history/performance-milestone-2026-09-01.md).

The current milestone is **measurement-ready, not performance-closed**.

Measurements are observations, not portable thresholds. Representation changes
must be compared on the same machine and toolchain and must retain semantic and
adversarial tests. The canonical closure workflow, not a hosted Criterion run,
owns acceptance verdicts.

## Canonical, exploratory, and comparison interfaces

Use the repository-owned single-revision runner for canonical observations:

```text
cargo xtask bench
```

Use Criterion 0.8.2 only for exploratory diagnostics:

```text
cargo xtask bench-explore -- --warm-up-time 3 evaluator
```

Criterion output never determines a closure verdict. Compile both systems and
their contract checks with:

```text
cargo xtask bench-check
```

Paired evidence uses:

```text
cargo xtask perf-compare --baseline 772f73a553f5806a365e29b799aa18a93bc0e515 \
  --candidate cfbc97f446826875bb13388ad4c0206bc3665c3b \
  --profile candidate --contract closure-v1 \
  --runner-manifest perf/runners/dol-perf-x64-01.toml \
  --artifact-dir target/perf/candidate
```

The comparator retains ordered samples and writes `metadata.json`,
`raw-samples.csv`, `summary.json`, and `paired-comparison.json` under the
`dol-perf/v1` schema. Candidate and release profiles refuse an unqualified or
mismatched runner. The A/A smoke command is cross-platform and timing-advisory:

```text
cargo xtask perf-a-a-smoke -- --artifact-dir target/perf/a-a-smoke
```

For targeted Linux evidence, use `cargo xtask perf-profile`; it collects
`perf stat`, flamegraph, assembly, DHAT, and RSS outputs where supported.

## Legacy suite details

The repository retains two framework-free optimized diagnostic targets:

- `dol-bench/roadmap` contains the archive-identified, user-attested historical
  fixture equivalents,
  the modern semantic workloads, engine decomposition, and scale sweeps;
- `dol-core/semantic_hot_paths` covers recursive type validation and canonical
  datum fingerprinting.

Use:

```text
cargo bench --package dol-bench --bench roadmap -- --suite historical --mode historical-fixed
cargo bench --package dol-bench --bench roadmap -- --suite modern --filter fingerprint
cargo bench --package dol-bench --bench roadmap -- --format csv --output target/benchmarks/dol.csv
cargo bench --package dol-core --bench semantic_hot_paths
cargo xtask bench-check
```

These direct targets remain useful for historical context and diagnosis. They
are not aliases for `cargo xtask bench`, which is reserved for the canonical
`closure-v1` runner.

The roadmap runner accepts:

- `--suite historical|modern|all`;
- `--filter <case-insensitive substring>`;
- `--mode historical-fixed|adaptive`;
- `--format text|csv`;
- `--output <path>`.

Adaptive mode calibrates each selected operation, then records nine samples of
approximately 150 ms. Historical-fixed mode performs one pass with the recorded
historical iteration count for each historical workload: 100,000 constructions,
1,000,000 evaluations, 100,000 clone/fingerprint pairs, and 10,000 model builds.
The historical operations retain their archived ordering. Every report includes
iterations, samples, median throughput and latency, observed range, Unix
timestamp, commit and dirty state, `rustc -Vv`, target/OS/CPU, power scheme,
declared repository bench-profile settings, observed Cargo profile environment
overrides, and archive/harness hashes.

The explicit bench profile uses one codegen unit, thin LTO, overflow checks, and
stripped debug information. Cargo ignores an explicit `panic` setting for test
and bench targets; its effective strategy remains the default unwind behavior.

## Provenance

The user supplied two archives. They are evidence and design history, not code
to import into the current implementation.

| Archive | SHA-256 | License in archive | Role and limitation |
| --- | --- | --- | --- |
| `dol-dev.zip` | `7938843901EF1D5C0F853139CDF4A8FE11032F6CDE0F98FCC3666F2BC71D5C39` | BSD-3-Clause, copyright Genius Tech Space | Oldest supplied starting point. It contains earlier packed-handle and workspace-design ideas, but a materially different API and no baseline harness. |
| `dol.zip` | `66F1631213557B6EB011D1D4CF896DFEC6C392F5B1D286175D6834BAB65FE71C` | MIT OR Apache-2.0, copyright Genius Tech Space | User-attested source/fixture snapshot associated with the August 23 baseline. It has no Git metadata, contains later files, and cannot identify the exact measured commit. |

The archived harness is `dol/benches/language.rs`, SHA-256
`03B2638B1F469A8F00FC0EF006E69515728290DDFF684F9684C934F2C4CA3306`.
Its behavior was independently expressed through current public APIs; archived
MIT/Apache implementation text was not copied. The current repository and all
new first-party code remain BSD-3-Clause.

### Archive replay result

The untouched `dol.zip` tree fails under Rust 1.98.0 before the harness can run:

1. the facade imports `BinaryPipeline`, `BinaryPipelineStage`, and
   `BinaryStageKind`, but `dol-core` does not re-export them at its crate root;
2. the archived MongoDB dev dependency fails `E0310` because
   `compile_pipeline<I, O>` calls a pipeline API requiring `I: 'static` and
   `O: 'static` without those bounds.

Two isolated copies were attempted: an archive-native copy with only the
recorded crate-root re-export repair, and a normalized copy with that repair plus
an explicit matching `[profile.bench]`. Both reached the independent MongoDB
error. The replay policy permits no archived engine, expression, pipeline,
runtime-model, PostgreSQL, or fixture edits, so both timing replays were aborted.
No archive-native result is claimed.

## Historical fixture contract

The historical suite reproduces these facts:

- `BenchmarkRow { id: u64, value: i32, active: bool, label: String }`;
- `(active == true AND value >= 10) AND label.contains("data")`;
- model name `benchmark_rows` (represented under the current API's required
  `historical/benchmark_rows` semantic key);
- fixed row `{ id: 1, value: 42, active: true, label: "data operating language" }`,
  whose expected result is `Truth::True`;
- `source -> filter -> id descending -> limit 100`.

Comparability is deliberately classified rather than implied:

| Historical label | Current result classification |
| --- | --- |
| Expression construction | Current-API equivalent fixture; the implementation and representation differ. |
| Expression evaluation | Prepared current-API equivalent. The archive used its older direct evaluator. |
| Expression/pipeline clone plus fingerprints | Current-API equivalent operation boundaries and fixed iteration count; semantic identity algorithms differ. |
| PostgreSQL capability analysis and planning | Not timed. Current exact PostgreSQL support rejects the fixture because sorting and text containment are outside its roadmap boundary. |
| Runtime model definition | Current `ModelBuilder::freeze` result reported separately. The removed JSON-driven `Model::define` API is not recreated for a benchmark. |
| `Expr`/`Pipeline` handle sizes | Directly comparable `size_of` measurements. |

The archived runner had no warm-up, repetition, range, or confidence estimate;
combined unrelated work in two labels; included JSON cloning in model timing;
and omitted CPU, target triple, OS build, power policy, and raw samples. Its
reported figures remain a local historical reference, not regression thresholds:

| Measurement | User-attested baseline |
| --- | ---: |
| Expression construction | 310,598 operations/s |
| Expression evaluation | 7,950,175 operations/s |
| Expression/pipeline clone plus fingerprints | 49,481 pairs/s |
| PostgreSQL capability analysis and planning | 68,660 plans/s |
| Runtime model definition | 97,468 models/s |
| `Expr` handle stack size | 32 bytes |
| `Pipeline` handle stack size | 32 bytes |

## Modern semantic suite

The modern suite preserves the earlier roadmap workload without presenting its
five percentages as historical regressions. It uses the four-field
`BenchAccount` model, the expression
`active == true AND balance >= 10 OR nickname is missing`, and a
source/filter/offset/limit pipeline. It now separates:

- expression preparation, direct prepare-plus-evaluate, and prepared evaluation;
- clone-only, cold/warm expression fingerprint, cold pipeline lowering, and
  cold/warm pipeline fingerprint;
- PostgreSQL placement, explain construction, SQL compilation, and complete
  supported placement-plus-compilation;
- expression depths 8/64/256, pipeline stages 4/32/256, and model widths
  4/64/1024.

The existing MongoDB compilation, wire decode, migration planning, exact vector
search, memory execution, and recursive semantic hot paths remain included.
Cold workloads create a fresh authoring root for each operation; warm workloads
prepare the shared root before sampling so caching cannot hide cold-path cost.

## Modern observations before this milestone

The August 31 results below are retained only as observations of the prior
modern semantic workload. They do not use the historical fixture and therefore
must not be described as direct baseline deltas.

| Measurement | Three-run median band |
| --- | ---: |
| Expression construction | 361,150-404,555 operations/s |
| Prepared expression evaluation | 964,528-1,117,340 operations/s |
| Expression/pipeline clone plus fingerprints | 17,326-19,835 pairs/s |
| PostgreSQL capability analysis and planning | 26,554-31,001 plans/s |
| Runtime model definition | 88,740-94,630 models/s |
| `Expr` handle stack size | 24 bytes |
| `Pipeline` handle stack size | 8 bytes |

An immutable export of commit `772f73a` was also measured five times on
September 1 with Rust 1.98.0 and the normalized profile before implementation
changes. Its independently calculated modern median bands were:

| Measurement | Five-run median band |
| --- | ---: |
| Expression construction | 407,677-491,445 operations/s |
| Prepared expression evaluation | 1,069,098-1,212,407 operations/s |
| Expression/pipeline clone plus fingerprints | 19,488-21,649 pairs/s |
| PostgreSQL capability analysis and planning | 30,377-33,880 plans/s |
| Runtime model definition | 104,772-114,476 models/s |
| PostgreSQL compilation | 10,759-12,336 compilations/s |
| MongoDB compilation | 1,121-1,199 compilations/s |
| Wire decode | 565,742-578,169 decodes/s |
| Migration plan | 87,298-99,045 plans/s |
| Exact vector search | 13,109-30,176 searches/s |
| Memory execution | 2,678-2,789 executions/s |

The broad vector range demonstrates why acceptance uses repeated same-host runs
and why raw ranges remain part of every report.

## Optimization acceptance

For a proposed low-risk optimization, compare five alternating same-host runs
against the immutable pre-change export. Accept it only when the target median
improves by at least 10%, exceeds observed run noise, and no related median
regresses by more than 5%. Semantic tests, fingerprints, and wire encodings are
hard gates; timing is not. Arenas, interning, or packed IR require the stricter
20% target gain, 30% prepared-node footprint reduction, semantic parity, and no
related regression above 5% before adoption.
