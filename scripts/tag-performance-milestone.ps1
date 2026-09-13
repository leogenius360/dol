[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$repositoryRoot = (& git rev-parse --show-toplevel).Trim()
if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($repositoryRoot)) {
    throw 'milestone tagging must run inside the DOL repository'
}
Set-Location -LiteralPath $repositoryRoot

$tag = 'perf-milestone-cfbc97f'
$candidate = 'cfbc97f446826875bb13388ad4c0206bc3665c3b'
$baseline = '772f73a553f5806a365e29b799aa18a93bc0e515'

& git cat-file -e "$candidate`^{commit}"
if ($LASTEXITCODE -ne 0) { throw "candidate commit is unavailable: $candidate" }
& git cat-file -e "$baseline`^{commit}"
if ($LASTEXITCODE -ne 0) { throw "baseline commit is unavailable: $baseline" }

& git rev-parse --quiet --verify "refs/tags/$tag" *> $null
if ($LASTEXITCODE -eq 0) {
    $actual = (& git rev-list -n 1 $tag).Trim()
    if ($LASTEXITCODE -ne 0) { throw "cannot resolve existing tag: $tag" }
    if ($actual -ne $candidate) {
        throw "$tag exists at $actual, expected $candidate; refusing to move it"
    }
    Write-Host "$tag already freezes $candidate"
    exit 0
}

$message = @'
DOL performance milestone before reproducible closure

Candidate: cfbc97f
Baseline:  772f73a

Freezes the optimized implementation used as the B-side for the
closure-v1 reproducible performance campaign.

This tag does not declare the performance milestone closed.
Closure requires dedicated-runner evidence and baseline ratification.
'@

& git tag --annotate $tag $candidate --message $message
if ($LASTEXITCODE -ne 0) { throw "failed to create $tag" }
Write-Host "created $tag at $candidate (candidate frozen; not performance-closed)"
