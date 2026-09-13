# closure-v1 candidate baseline (not ratified)

No numerical baseline is committed yet. The `dol-perf-x64-01` manifest is
unqualified, so adding synthetic or hosted-run timing here would misrepresent
the evidence state.

After runner qualification and review, this directory may contain only:

```text
metadata.json
raw-samples.csv
summary.json
paired-comparison.json
README.md
heap-summary.json        # only when needed for the closure decision
```

Large profiling captures remain CI artifacts. The accepted bundle must identify
both original lockfile hashes, the exact toolchain, source commits, fixture
manifest, runner, raw observations, retries, and final finite classification.
