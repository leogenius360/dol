# Baseline-Identified Performance Milestone Report

> **Historical snapshot.** This report records the optimization milestone at
> candidate revision `cfbc97f` on 2026-09-01. It is not closure-v1 acceptance;
> current status and evidence rules live in the
> [reproducible closure contract](../performance/closure-v1.md).

## Outcome

This follow-up milestone starts from clean commit `772f73a` and leaves roadmap
stages A-M unchanged. It authenticates the supplied benchmark lineage as far as
the available evidence permits, adds reproducible historical and modern suites,
and retains only optimizations that satisfy the milestone's correctness and
performance policy.

The accepted final implementation preserves canonical fingerprints, wire
encoding, expression semantics, and the BSD-3-Clause license. On the final
five-run sweep, the principal modern workload medians improved by 59.0% for
prepared expression evaluation, 64,744% for clone plus warm fingerprints,
2,080.9% for PostgreSQL capability analysis/planning, and 17.0% for memory
pipeline execution. Expression construction remained inside the related 5%
guardrail at -4.1%.

## Repository and provenance review

The repository is a 14-package Rust workspace plus the isolated fuzz package.
The facade delegates semantic authoring and planning to `dol-core`, shared
engine policy to `dol-engine`, four engine adapters to `engines/`, and migration,
wire, ML, conformance, benchmark, and repository-task concerns to their own
packages. The strict source metric reports 176 Rust files and 43,929 lines; only
`engines/mongodb/src/compiler.rs` exceeds the repository's 800-line architecture
review threshold, at 1,082 lines.

The benchmark investigation used these user-supplied archives:

| Evidence | SHA-256 | Finding |
| --- | --- | --- |
| `dol-dev.zip` | `7938843901EF1D5C0F853139CDF4A8FE11032F6CDE0F98FCC3666F2BC71D5C39` | Oldest supplied design/start point; BSD-3-Clause; materially different API; no baseline harness. |
| `dol.zip` | `66F1631213557B6EB011D1D4CF896DFEC6C392F5B1D286175D6834BAB65FE71C` | User-attested August 23 baseline snapshot; MIT OR Apache-2.0; no Git identity and mixed later files. |
| `dol/benches/language.rs` inside `dol.zip` | `03B2638B1F469A8F00FC0EF006E69515728290DDFF684F9684C934F2C4CA3306` | Exact historical fixture and fixed iteration ordering used for the equivalent suite. |

The archived fixture is now locked by test: model name `benchmark_rows`, fields
`id: u64`, `value: i32`, `active: bool`, and `label: String`; expression
`(active == true AND value >= 10) AND label.contains("data")`; row
`{ id: 1, value: 42, active: true, label: "data operating language" }`; and
pipeline `source -> filter -> id descending -> limit 100`. The current API
requires a namespaced semantic key, so the equivalent model uses
`historical/benchmark_rows` without claiming identical historical identity.

### Archive replay

The untouched archive was compiled out of tree with Rust 1.98.0, locked
dependencies, all features, and no network. It failed before benchmarking
because the facade imports three `BinaryPipeline` symbols that `dol-core` does
not re-export. Archive-native and normalized-profile copies were then attempted
with only the permitted crate-root re-export repair; the normalized copy also
received the permitted benchmark profile. Both reached an independent archived
MongoDB `E0310` lifetime error.

The approved replay policy did not allow changing archived engine dependencies
or implementation code, so replay stopped there. No native replay timing is
claimed. The historical numbers remain user-attested, not commit-pinned.

## Implementation by subsystem

### Workspace metadata and build profile

- Central workspace metadata now defines author `Dominic Maabobra Tuolong`,
  repository `https://github.com/leogenius360/dol`, and license
  `BSD-3-Clause`; every workspace package inherits it and the isolated fuzz
  package carries the same explicit values.
- `[profile.bench]` now declares one codegen unit, thin LTO, overflow checks,
  and stripped debug information. Cargo's effective test/bench panic strategy
  remains unwind because Cargo ignores a per-profile panic override for those
  targets.
- `cargo metadata --no-deps --format-version 1` confirms the author,
  repository, and BSD-3-Clause license for all 14 workspace packages.

### Expression evaluator and identity

- Prepared evaluation no longer allocates `Vec<Option<Datum>>` per call or
  clones the root result into that memo. Preparation assigns a unique node ID
  to every occurrence, so the removed memo could never serve a reusable value.
- Eager evaluation order, short-circuit/coalesce behavior, per-node semantic
  validation, scope checks, parameters, custom functions, conditionals,
  membership, and existential resolution remain unchanged.
- Successful normalized expression fingerprints are cached in `OnceLock`
  storage inside the Arc-owned expression node. Failures remain uncached.
- The public `Expr<T>` handle remains 24 bytes.

### Pipeline preparation and logical plans

- Arc-owned pipeline nodes now hold immutable stage data plus a `OnceLock` for
  the successful default-limit logical plan.
- New `Pipeline::prepared_plan() -> Result<Arc<LogicalPlan>>` exposes the shared
  plan. `logical_plan()` remains the owned compatibility API and clones the
  cached plan; `logical_plan_with_limits()` always performs an uncached custom
  lowering, including when callers explicitly pass default-shaped limits.
- Pipeline fingerprints use the prepared plan, and engine convenience paths use
  the shared plan. Failed preparations are not cached.
- The redundant node-limit prepass was removed; the existing topological
  collection enforces the same limit during lowering.
- Logical source nodes borrow the model's static definition rather than cloning
  it into every source node. Plan output remains owned for compatibility.
- The public `Pipeline<T>` handle remains 8 bytes.

### Engine paths

- JSONL, memory, MongoDB, and PostgreSQL pipeline convenience methods prepare
  once and share the resulting plan.
- `PostgresEngine::explain_plan(&LogicalPlan, PlacementPolicy)` separates plan
  preparation from placement/explain work; the pipeline overload delegates to
  it.
- Static explain details use borrowed strings and allocate only a dynamic
  unsupported reason.
- PostgreSQL and MongoDB registration-time rejection and repeated exact-model
  lookup tests were added. Their public mapping behavior remains unchanged.
- A borrowed engine-expression view and lookup-without-revalidation candidate
  was implemented and tested across PostgreSQL and MongoDB, then reverted
  because compiler-only improvements did not reach the 10% acceptance bar.

### Benchmark and repository tooling

- `dol-bench/roadmap` now has `historical`, `modern`, and `all` suites;
  historical-fixed and nine-sample adaptive modes; case-insensitive filters;
  text/CSV formats; and explicit output paths.
- Reports include the commit/dirty state, Unix timestamp, `rustc -Vv`, target,
  Windows build, CPU, power scheme, declared repository bench profile, observed
  `CARGO_PROFILE_BENCH_*` overrides, archive hashes, sample counts, iterations,
  medians, and ranges.
- Historical-only selection no longer initializes modern fixtures, and vice
  versa. Cargo's appended `--bench` marker is accepted. `--help` exits normally.
- Unit tests cover option parsing, invalid/missing values, suite/filter
  selection, historical fixed-iteration behavior, and CSV escaping.
- The modern suite separates preparation, direct preparation/evaluation,
  prepared evaluation, clone-only, cold/warm identity, lowering, placement,
  explain construction, compilation, and supported placement-plus-compilation.
  It also adds depth/stage/width sweeps and retains wire, migration, ML, memory,
  MongoDB, and recursive semantic workloads.

## API migration notes

Most consumers require no change. Existing `Pipeline::logical_plan()` callers
still receive an owned `LogicalPlan`, and engine `explain`/`execute` signatures
are unchanged.

Repeated callers should prefer:

```rust
let plan = pipeline.prepared_plan()?;
let explain = postgres.explain_plan(&plan, PlacementPolicy::RemoteOnly)?;
```

Engine authors matching `LogicalNode::Source` must account for its `model` field
changing from `Box<ModelDef>` to `&'static ModelDef`; remove ownership-dependent
cloning/dereferencing where possible. This is the only retained engine-facing
representation change. The experimental iterator-based `engine_view()` API was
reverted, so its prior slice/`Vec<usize>` adapter contract remains intact.

No semantic fingerprint version, wire format, diagnostic code, or public handle
layout changed.

## Benchmark method and host

The immutable before tree was exported from `772f73a`; the after tree was dirty
only with this milestone. Measurements used Rust 1.98.0
(`88d9e12ae178fab0fb5cc050a94da85685d449ea`), target
`x86_64-pc-windows-msvc`, Windows `10.0.26200.6899`, AMD64 Family 23 Model 17
Stepping 0, and the Balanced power scheme. The explicit normalized bench profile
was used on both trees.

Five same-host before/after pairs were alternated for the candidate build. After
the subthreshold engine-view candidate was reverted, five additional full final
runs confirmed the retained targets. Timing is informational; medians are not a
CI threshold.

### Primary modern evidence

Values are operations per second. Ranges are the minimum and maximum of the five
run medians, not the nine samples inside one run.

| Workload | Before median (range) | Final median (range) | Delta |
| --- | ---: | ---: | ---: |
| Expression construction | 487,598 (423,832-501,581) | 467,705 (460,940-478,172) | -4.1% |
| Prepared expression evaluation | 1,208,761 (1,188,170-1,213,390) | 1,922,412 (1,877,640-1,961,430) | +59.0% |
| Expression/pipeline clone plus fingerprints | 22,612 (18,723-23,011) | 14,662,520 (13,732,909-14,762,519) | +64,744% |
| PostgreSQL capability analysis/planning | 31,037 (28,784-31,128) | 676,879 (667,067-685,647) | +2,080.9% |
| Runtime model definition | 107,070 (98,091-108,874) | 109,683 (105,310-110,558) | +2.4% |
| PostgreSQL SQL compilation | 11,230 (9,755-12,083) | 11,169 (10,368-11,702) | -0.5% |
| MongoDB offline compilation | 1,140 (1,101-1,166) | 1,123 (1,024-1,156) | -1.5% |
| Wire decode | 582,280 (552,601-584,187) | 521,268 (488,021-531,768) | -10.5% |
| Migration plan | 93,164 (91,246-100,047) | 93,491 (89,546-94,442) | +0.4% |
| Exact vector search | 30,088 (13,221-30,273) | 29,433 (28,701-29,709) | -2.2% |
| Memory execution | 2,814 (2,719-2,845) | 3,291 (3,196-3,345) | +17.0% |

The wire path was not modified. Its later final sweep ran below both its earlier
alternating after median (558,675, -4.1%) and the before tree while other
unmodified workloads moved in both directions; it is recorded as host/run
variation, not hidden or attributed to a wire change. It is unrelated to the
accepted evaluator, identity, plan-cache, and memory-path targets.

Raw five-run primary series, in execution order:

| Workload | Before series | Final series |
| --- | --- | --- |
| Construction | 496180, 455630, 487598, 501581, 423832 | 460940, 468216, 467705, 478172, 467315 |
| Evaluation | 1209970, 1208761, 1188170, 1213390, 1203976 | 1877640, 1961430, 1940543, 1897670, 1922412 |
| Clone/fingerprints | 22470, 23011, 18723, 22612, 22968 | 14762519, 14746834, 14623385, 14662520, 13732909 |
| PostgreSQL planning | 31037, 31061, 28784, 31128, 30387 | 667067, 678849, 676879, 685647, 673507 |
| Model definition | 108810, 107070, 108874, 103656, 98091 | 105310, 110325, 109649, 110558, 109683 |
| PostgreSQL compile | 10672, 11663, 9755, 11230, 12083 | 11527, 11702, 11169, 10670, 10368 |
| MongoDB compile | 1101, 1145, 1121, 1140, 1166 | 1120, 1123, 1129, 1156, 1024 |
| Wire decode | 552601, 582280, 583950, 584187, 581442 | 521268, 524044, 531768, 488021, 499135 |
| Migration | 91246, 98937, 91563, 100047, 93164 | 93491, 91749, 94442, 94423, 89546 |
| Vector search | 29728, 30273, 13221, 30196, 30088 | 29626, 28870, 29433, 29709, 28701 |
| Memory execution | 2811, 2814, 2837, 2719, 2845 | 3337, 3282, 3291, 3196, 3345 |

### Final decomposed medians

| Workload | Median ops/s | Five-run range |
| --- | ---: | ---: |
| Expression preparation | 51,301 | 43,719-52,356 |
| Direct prepare plus evaluate | 48,814 | 45,883-49,278 |
| Clone only | 28,375,868 | 27,762,664-28,791,094 |
| Cold / warm expression fingerprint | 51,878 / 138,058,368 | 51,071-52,115 / 129,390,295-144,321,999 |
| Cold / warm pipeline lowering | 30,987 / 55,225,837 | 29,108-33,052 / 53,806,337-57,484,646 |
| Cold / warm pipeline fingerprint | 33,190 / 51,078,267 | 31,706-34,116 / 48,122,403-51,547,985 |
| PostgreSQL placement | 713,257 | 679,795-753,107 |
| PostgreSQL explain construction | 9,989,317 | 9,826,255-10,550,787 |
| PostgreSQL supported placement plus compilation | 12,111 | 11,335-12,631 |

Scale medians were 70,166/9,892/2,552 operations/s for expression depths
8/64/256; 102,456/26,370/3,982 for pipeline stages 4/32/256; and
112,085/7,981/469 for model widths 4/64/1024.

### Corrected historical equivalent results

These are current-API equivalents, not direct regressions against the archived
implementation:

| Workload | Adaptive median (range) | Historical-fixed one-pass result |
| --- | ---: | ---: |
| Construction | 287,410 (187,772-331,343) | 356,846 |
| Prepared evaluation | 1,460,809 (1,271,258-1,585,385) | 1,403,557 |
| Clone plus fingerprints | 13,943,210 (11,191,782-15,439,405) | 13,576,995 |
| `ModelBuilder::freeze` | 113,636 (81,608-121,239) | 119,124 |

The historical PostgreSQL fixture is not timed: current exact placement rejects
its sort/text-containment surface with `ENGINE-PLACEMENT-001`. The removed JSON
`Model::define` operation is not recreated. Direct handle comparisons improve
from 32 to 24 bytes for `Expr` and from 32 to 8 bytes for `Pipeline`.

## Accepted, rejected, and deferred work

Accepted:

- evaluator memo/result-clone removal;
- successful expression fingerprint caching;
- successful default pipeline plan caching and shared engine use;
- redundant lowering traversal removal;
- static source model references;
- shared PostgreSQL plan-explain API and allocation-free static explain text.

Rejected and reverted:

- selective boundary/root-only evaluator validation: targeted five-run median
  2,101,654 eval/s versus 2,106,927 with full validation (-0.25%);
- borrowed engine-view iterators plus per-lookup mapping validation removal:
  candidate PostgreSQL compile improved 8.4% and MongoDB 3.9%, both below 10%.

Deferred:

- a private borrowed/owned evaluator datum representation, because no CPU sample
  established owned conversion at the required 20% share and the lower-risk
  evaluator change already cleared its target;
- arenas, interning, and packed IR, because no remaining accepted target met the
  stricter 20% gain plus 30% footprint trigger. No semantic fingerprint or wire
  representation was changed speculatively.

## Verification record

| Command/gate | Result |
| --- | --- |
| `cargo xtask check` | Passed formatting, all-target/all-feature checks, isolated fuzz checks, Clippy with warnings denied, all workspace tests and doctests, rustdoc with warnings denied, facade feature matrix, adoption examples, and metrics. |
| `cargo xtask bench-check` | Passed for `semantic_hot_paths` and `roadmap` benchmark targets. |
| `cargo test -p dol-bench --lib` | 4/4 benchmark-tooling tests passed. |
| `cargo test --test performance_milestone` | 4/4 historical fixture/cache/concurrency tests passed. |
| PostgreSQL/MongoDB offline suites | Passed, including unchanged compiler snapshots, mapping rejection, and repeated lookup. |
| `cargo xtask postgres-live` | Unavailable: Docker engine was not running on this host. |
| MongoDB external conformance | Not run: `DOL_MONGODB_TEST_URL` was not configured. |
| `cargo metadata --no-deps --format-version 1` | All workspace packages report the intended author, repository, and BSD-3-Clause license. |
| `git diff --check` | Passed. |

The database live gates remain environmental follow-ups, not silently treated as
passes. Start Docker and rerun `cargo xtask postgres-live`; configure
`DOL_MONGODB_TEST_URL` and rerun `cargo xtask mongodb-live-external` when those
services are available.
