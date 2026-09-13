# Reproducible performance closure

## Current status

DOL is **measurement-ready**, not performance-closed.

The repository now has a versioned workload contract, deterministic comparison
rules, immutable candidate lineage, canonical fixtures, and automation for
collecting evidence. The dedicated runner is not yet qualified and no
`closure-v1` baseline has been ratified. Hosted timing, local timing, historical
observations, and an unqualified runner cannot close the milestone.

The governing rule is:

> No qualifying evidence, no runtime optimization.

## Frozen lineage and tag meanings

`closure-v1` compares these complete Git identities:

```text
baseline:  772f73a553f5806a365e29b799aa18a93bc0e515
candidate: cfbc97f446826875bb13388ad4c0206bc3665c3b
```

The annotated `perf-milestone-cfbc97f` tag freezes the optimized candidate used
as the B side. It means “candidate frozen,” not “performance accepted.” The
future `perf-baseline-v1` tag may be created only after dedicated-runner
evidence is reviewed and the closure report is complete.

## Milestone boundaries

The work has three independently reviewable increments:

1. Measurement infrastructure adds workload, statistics, artifact, fixture,
   CI, and profiling support without changing runtime behavior. Its terminal
   state is measurement-ready.
2. Dedicated-runner evidence qualifies the machine, executes A/A calibration,
   compares the frozen revisions, classifies wire decode, and records memory
   behavior. Measurements are artifacts rather than runtime commits.
3. Evidence-backed correction changes runtime source only if a failing workload
   qualifies, then reruns the complete campaign before baseline ratification.

Without qualifying `closure-v1` evidence, this milestone does not authorize:

- evaluator representation redesign;
- plan-cache synchronization redesign;
- fingerprint/hash algorithm changes;
- arena adoption;
- interning redesign; or
- packed IR.

Arena or similarly invasive representation work additionally requires at least
20% meaningful performance improvement and at least 30% prepared-representation
footprint reduction, with semantic parity and no related regression above 5%.

## Workload ABI

[`perf/contracts/closure-v1.toml`](../../perf/contracts/closure-v1.toml) is the
normative contract. It freezes:

- stable scenario IDs and separate lifecycle labels;
- construction, setup, preparation, and sampling boundaries;
- semantic fixture inputs, expected fingerprints, and exact wire-v1 bytes;
- work-unit definitions and required scenarios;
- five sampling profiles and the Rust 1.98.0 canonical target;
- permitted benchmark-only overlays; and
- statistical decision, noise, retry, memory, and closure rules.

It does not freeze display names, compatible output presentation, profiler
implementation, or additive metadata that preserves `dol-perf/v1`
compatibility. A semantic fixture, boundary, profile, or decision-rule change
requires `closure-v2` (or another explicit contract/profile version); it must
not silently rewrite `closure-v1`.

Scenario IDs are machine-oriented and are never derived from prose. For
example, cold and warm identity observations share `expr.identity` and differ
in their `lifecycle` field. The contract registry is authoritative for both the
canonical and scaling suites.

## Commands

```text
cargo xtask bench
cargo xtask bench-explore -- <criterion arguments>
cargo xtask bench-check
cargo xtask perf-compare --baseline <revision> --candidate <revision> \
  --profile candidate|release --contract closure-v1 \
  --runner-manifest perf/runners/dol-perf-x64-01.toml \
  --artifact-dir <directory>
cargo xtask perf-a-a-smoke -- --artifact-dir <directory>
cargo xtask perf-profile --commit <revision> --scenario <scenario-id> \
  --artifact-dir <directory>
```

`bench` is the DOL-owned canonical single-revision runner. With
`--artifact-dir`, its CSV/JSON artifact files contain only machine data and all
diagnostics go to stderr.
`bench-explore` runs Criterion 0.8.2 for diagnostic workload exploration and is
never a source of a closure verdict.

The comparator creates detached worktrees and uses each side’s original
`Cargo.lock`. Both lockfile hashes are recorded. Dependency changes therefore
remain visible as part of a revision comparison while Rust and the target stay
normalized.

An identical fixture overlay is allowed only in the paths recorded by the
contract. The artifact records the overlay patch hash, allowed paths, and actual
paths and requires `actual_paths` to be a subset of `allowed_paths`. Any overlay
under DOL runtime source, including `crates/**/src/**` or `engines/**/src/**`,
invalidates the comparison.

## Immutable sampling profiles

| Profile | Warm-up | Samples | Target per sample | Pairs |
| --- | ---: | ---: | ---: | ---: |
| `historical-fixed` | none | fixed | fixed | none |
| `adaptive` | 1 s | 9 | about 150 ms | none |
| `smoke` | 250 ms | 5 | about 50 ms | 2 |
| `candidate` | 3 s | 20 | about 300 ms | 7 |
| `release` | 3 s | 30 | about 500 ms | 10 |

Candidate/release runs require a strictly matching, qualified runner manifest.
The hosted smoke profile exercises the same contract machinery but its timing
is advisory.

## Evidence model

Every comparison emits a `dol-perf/v1` bundle with rigid responsibilities:

- `metadata.json` contains identity and provenance only: schema/contract,
  runner, source and lockfile hashes, fixture/overlay hashes, `rustc -Vv`, Cargo,
  target/profile/flags, CPU topology, affinity, SMT/governor/boost/NUMA,
  environment, kernel/microcode when available, timestamp, and bootstrap seed.
- `raw-samples.csv` contains ordered observations only: pair, side, order,
  scenario/lifecycle, sample, iterations, elapsed/cost/throughput/work units,
  Tukey class, comparison eligibility, and rejection reason. Outliers and
  rejected attempts are never deleted.
- `summary.json` contains one-side median, p10, p90, MAD, normalized MAD, CV,
  min/max, sample count, and outlier counts.
- `paired-comparison.json` contains baseline/candidate identities, pair count,
  paired cost ratios, deterministic bootstrap interval, retry status, finite
  classification, and reason.

Serialized classifications are exactly `improvement`, `neutral`, `warning`,
`hard-regression`, `noisy`, `inconclusive`, and `invalid-comparison`; generic
pass/fail labels are not evidence.

## Statistics and retry policy

All decisions use the cost axis:

```text
candidate ns/op
--------------- - 1
baseline ns/op
```

Positive is slower and negative is faster. The paired bootstrap uses the
contract’s 32-byte serialized seed and exactly 100,000 resamples.

- A hard regression requires median cost increase of at least 5% and the paired
  95% interval’s lower bound above +3%.
- A warning begins at a 3% median cost increase when the hard rule is not met.
- A retained improvement requires median improvement of at least 10%, the 95%
  interval upper bound below zero, and a workload that was genuinely failing
  before the fix.
- A side is noisy when NMAD exceeds 5%, CV exceeds 10%, or Tukey outliers exceed
  20% of observations. NMAD is `MAD / abs(median)` and CV uses population
  standard deviation over `abs(mean)`.

Noise permits exactly one retry of the complete temporal pair. If A4 is noisy
and B4 appears valid, both A4 and B4 are rejected for comparison and A4′/B4′
are measured in the scheduled order. The rejected observations remain in raw
samples with `valid_for_comparison=false` and a reason.

## Exact semantic and layout fixtures

The committed 84-byte wire fixture has independent one-way assertions:

```text
canonical Vec<Option<String>> TypeDef -> pinned wire-v1 bytes
pinned wire-v1 bytes -> canonical Vec<Option<String>> TypeDef
```

This prevents matching encoder and decoder changes from silently redefining the
wire protocol. The historical expression likewise maps to a separately pinned
32-byte canonical fingerprint; it is not generated by the test.

On x86-64, public `Expr<Truth>` and `Pipeline<T>` handles are hard-pinned to 24
and 8 bytes. Private `ExprNode` and `PipelineNode` compiler measurements are
review gates rather than API. Their current 240-byte/16-byte-aligned and
280-byte/8-byte-aligned values are documented in
[`perf/FOOTPRINT.md`](../../perf/FOOTPRINT.md). A private change requires an
explicit pin update, footprint analysis, and benchmark evidence.

## Memory policy

There is no relative heap cap against `772f73a`; the candidate intentionally
adds useful caches. Evidence must instead show bounded retained state,
post-owner-drop return, approximately linear normalized bytes per node/stage,
and no more than 25% large-versus-small normalized growth. The required heap,
RSS, allocation, and normalized fields are listed in the contract.

## Runner authority and CI trust

The four trust levels are intentionally separate:

| Trigger | Work | Trust |
| --- | --- | --- |
| Pull request | correctness + benchmark contract | Hard correctness gate |
| Pull request / hosted | A/A performance smoke | Workload hard; timing advisory |
| Manual dispatch | self-hosted comparison | Evidence authority after qualification |
| Nightly / weekly | candidate, release, profiling | Enabled only after qualification |

Persistent performance workers run only trusted repository refs, with a
read-only token, no repository secrets, no arbitrary pull-request execution,
and labels `[self-hosted, linux, x64, dol-perf]`. Authoritative manual and
scheduled comparisons use `--enforce`; their complete evidence is uploaded even
when a non-closing verdict makes the job fail.

The runner manifest is benchmark ABI. Live model, selected physical CPU,
affinity availability, NUMA, SMT, governor, boost, toolchain, OS, and target
must match exactly; canonical measurement refuses a mismatch rather than
warning. The manifest must also carry a non-placeholder reviewed isolation
policy and require one concurrent job, while workflow concurrency serializes
authoritative runs.

Qualification has two gates:

1. at least 20 complete A/A comparisons over at least 14 calendar days with
   zero false hard regressions; and
2. repeated `772f73a` ↔ `cfbc97f` runs where established major signals preserve
   sign and approximate magnitude without unexplained verdict oscillation.

The committed manifest remains unqualified until real provisioning and
calibration records replace its explicit sentinels.

## Closure decision

Closure requires all of the following on the evidence-authority runner:

- prepared evaluation at least 30% faster than the baseline;
- warm identity and PostgreSQL shared-plan gains each retained by at least 10%;
- no confirmed cold-path or collateral regression above 5%;
- wire decode classified as neutral, explicitly accepted, or fixed—never left
  unexplained;
- exact semantics, diagnostic codes, canonical fingerprints, wire-v1 bytes,
  public layouts, and fixture identity preserved;
- bounded cache growth, post-drop return, and at most 25% large-scale normalized
  memory growth demonstrated; and
- complete provenance and ordered raw observations retained.

Wire decode remains first in the conditional-fix queue because prior same-host
observations showed a signal above the project guardrail. If it still fails,
collect `perf stat`, flamegraph, assembly, DHAT, and RSS evidence before editing
wire code. Expression construction is investigated only if new canonical
evidence crosses 5%; other runtime changes require a newly confirmed diagnostic
failure.

Until the checked report in `perf/reports/PERFORMANCE_CLOSURE_V1.md` is complete
and `perf-baseline-v1` is ratified, the only truthful state is
**measurement-ready, not performance-closed**.
