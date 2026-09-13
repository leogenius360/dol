# DOL performance evidence

This directory contains the immutable workload contract and the small,
reviewable part of DOL performance evidence. It is not a claim that the
performance milestone is closed.

The current state is **measurement-ready**:

- `contracts/closure-v1.toml` freezes lineage, fixtures, scenario identities,
  sampling profiles, statistical rules, and closure criteria;
- `fixtures/closure-v1/` contains independently pinned semantic, fingerprint,
  and wire-v1 inputs;
- `runners/` contains the strict dedicated-runner manifest. The committed
  runner is deliberately marked unqualified until its calibration record is
  complete;
- `baselines/` reserves the minimal ratified-evidence layout; and
- `reports/PERFORMANCE_CLOSURE_V1.md` is the evidence report that must be
  completed before baseline ratification.

Canonical comparisons emit a `dol-perf/v1` artifact bundle:

```text
metadata.json
raw-samples.csv
summary.json
paired-comparison.json
```

Raw profiler output (`perf.data`, flamegraphs, DHAT captures, RSS traces, and
assembly dumps) belongs in durable CI artifacts, not Git. A committed baseline
contains only the files needed to reproduce a verdict and, when applicable, a
small `heap-summary.json`.

`perf-milestone-cfbc97f` freezes the candidate revision. It does not accept its
performance. Only reviewed dedicated-runner evidence may justify the separate
`perf-baseline-v1` tag.
