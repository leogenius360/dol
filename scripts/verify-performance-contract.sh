#!/usr/bin/env sh
set -eu

repository_root=$(git rev-parse --show-toplevel)
cd "$repository_root"

cargo_command=${CARGO:-cargo}
contract=perf/contracts/closure-v1.toml
fixture_root=perf/fixtures/closure-v1
manifest=$fixture_root/MANIFEST.sha256

fail() {
    printf '%s\n' "performance contract verification failed: $*" >&2
    exit 1
}

require_literal() {
    file=$1
    literal=$2
    grep -F -- "$literal" "$file" >/dev/null || fail "$file is missing: $literal"
}

normalized_sha256() {
    tr -d '\r' < "$1" | sha256sum | awk '{print $1}'
}

test -f "$contract" || fail "missing $contract"
test -f "$manifest" || fail "missing $manifest"
require_literal "$contract" 'schema = "dol-perf-contract/v1"'
require_literal "$contract" 'contract = "closure-v1"'
require_literal "$contract" 'evidence_schema = "dol-perf/v1"'
require_literal "$contract" 'state = "measurement-ready"'
require_literal "$contract" 'baseline_commit = "772f73a553f5806a365e29b799aa18a93bc0e515"'
require_literal "$contract" 'candidate_commit = "cfbc97f446826875bb13388ad4c0206bc3665c3b"'
require_literal "$contract" 'bootstrap_resamples = 100000'
require_literal "$contract" 'retry_complete_pair_once = true'

while IFS='  ' read -r expected relative; do
    test -n "$expected" || continue
    test -n "$relative" || fail "invalid fixture manifest row"
    path=$fixture_root/$relative
    test -f "$path" || fail "missing fixture $path"
    actual=$(normalized_sha256 "$path")
    test "$actual" = "$expected" || fail "fixture hash mismatch: $relative"
done < "$manifest"

expected_manifest=$(sed -n 's/^fixture_manifest_sha256 = "\([0-9a-f][0-9a-f]*\)"$/\1/p' "$contract")
test -n "$expected_manifest" || fail "contract has no fixture_manifest_sha256"
test "${#expected_manifest}" -eq 64 || fail "fixture_manifest_sha256 must contain 64 hex characters"
actual_manifest=$(normalized_sha256 "$manifest")
test "$actual_manifest" = "$expected_manifest" || fail "fixture manifest hash mismatch"

"$cargo_command" fmt --all --check
"$cargo_command" check --workspace --all-targets --all-features
"$cargo_command" clippy --workspace --all-targets --all-features -- -D warnings
"$cargo_command" test --workspace --all-features
"$cargo_command" test --package dol-bench
"$cargo_command" test --package xtask
"$cargo_command" xtask bench-check
"$cargo_command" test --test performance_milestone
"$cargo_command" test --package dol-wire --test wire
"$cargo_command" test --package dol-core performance_layout_contract
aa_root=$(mktemp -d "${TMPDIR:-/tmp}/dol-perf-a-a-verify.XXXXXX")
trap 'rm -rf -- "$aa_root"' EXIT HUP INT TERM
"$cargo_command" xtask perf-a-a-smoke -- --artifact-dir "$aa_root/artifacts"

printf '%s\n' 'performance contract verification passed (measurement-ready; not performance-closed)'
