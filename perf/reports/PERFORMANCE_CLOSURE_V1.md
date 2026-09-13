# closure-v1 performance evidence report

Status: **measurement-ready; evidence pending; not performance-closed**

Candidate: `cfbc97f446826875bb13388ad4c0206bc3665c3b`

Baseline: `772f73a553f5806a365e29b799aa18a93bc0e515`

Contract: `closure-v1`

Evidence schema: `dol-perf/v1`

## Preconditions

- [ ] `dol-perf-x64-01` has at least 20 clean A/A comparisons over at least 14
      calendar days and zero false hard regressions.
- [ ] Repeated known A/B calibration preserves the sign and approximate
      magnitude of established signals without unexplained verdict oscillation.
- [ ] The runner manifest is complete, reviewed, and marked qualified.
- [ ] Candidate/release comparisons use Rust 1.98.0, the canonical target,
      pinned affinity, and each revision's original `Cargo.lock`.

## Required evidence

| Requirement | Artifact/result | Status |
| --- | --- | --- |
| Prepared evaluation is at least 30% faster | Pending | Not measured |
| Warm identity retains at least 10% gain | Pending | Not measured |
| PostgreSQL warm/shared-plan retains at least 10% gain | Pending | Not measured |
| No confirmed cold-path regression above 5% | Pending | Not measured |
| No collateral regression above 5% | Pending | Not measured |
| Wire decode is neutral, accepted with explanation, or fixed | Pending | Not classified |
| Exact semantics/fingerprint/wire/diagnostic/layout contracts pass | Pending | Not verified on evidence runner |
| Cache growth is bounded and post-drop return is demonstrated | Pending | Not measured |
| Large-scale normalized memory growth is at most 25% | Pending | Not measured |

## Wire investigation

If the paired campaign confirms a wire-decode regression, collect `perf stat`,
flamegraph, assembly, DHAT, and RSS evidence before authorizing a runtime edit.
The unchanged-source observation is not an explanation by itself.

## Decision

No closure decision has been made. Do not create `perf-baseline-v1` until every
required item above has reviewed dedicated-runner evidence and this report links
the accepted minimal artifact bundle.
