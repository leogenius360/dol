#!/usr/bin/env sh
set -eu

repository_root=$(git rev-parse --show-toplevel)
cd "$repository_root"

tag=perf-milestone-cfbc97f
candidate=cfbc97f446826875bb13388ad4c0206bc3665c3b
baseline=772f73a553f5806a365e29b799aa18a93bc0e515

git cat-file -e "$candidate^{commit}"
git cat-file -e "$baseline^{commit}"

if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
    actual=$(git rev-list -n 1 "$tag")
    test "$actual" = "$candidate" || {
        printf '%s\n' "$tag exists at $actual, expected $candidate; refusing to move it" >&2
        exit 1
    }
    printf '%s\n' "$tag already freezes $candidate"
    exit 0
fi

message='DOL performance milestone before reproducible closure

Candidate: cfbc97f
Baseline:  772f73a

Freezes the optimized implementation used as the B-side for the
closure-v1 reproducible performance campaign.

This tag does not declare the performance milestone closed.
Closure requires dedicated-runner evidence and baseline ratification.'

git tag --annotate "$tag" "$candidate" --message "$message"
printf '%s\n' "created $tag at $candidate (candidate frozen; not performance-closed)"
