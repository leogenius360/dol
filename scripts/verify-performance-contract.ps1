[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$repositoryRoot = (& git rev-parse --show-toplevel).Trim()
if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($repositoryRoot)) {
    throw 'performance contract verification must run inside the DOL repository'
}
Set-Location -LiteralPath $repositoryRoot

$cargoCommand = if ($env:CARGO) { $env:CARGO } else { 'cargo' }
$contract = 'perf/contracts/closure-v1.toml'
$fixtureRoot = 'perf/fixtures/closure-v1'
$manifest = Join-Path $fixtureRoot 'MANIFEST.sha256'

function Assert-Literal {
    param([string]$Path, [string]$Literal)
    if (-not (Select-String -LiteralPath $Path -SimpleMatch $Literal -Quiet)) {
        throw "$Path is missing: $Literal"
    }
}

function Get-NormalizedSha256 {
    param([string]$Path)
    $text = [System.IO.File]::ReadAllText((Resolve-Path -LiteralPath $Path))
    $normalized = $text.Replace("`r`n", "`n").Replace("`r", "`n")
    $utf8 = New-Object System.Text.UTF8Encoding($false)
    $sha = [System.Security.Cryptography.SHA256]::Create()
    try {
        return (($sha.ComputeHash($utf8.GetBytes($normalized)) | ForEach-Object {
            $_.ToString('x2')
        }) -join '')
    }
    finally {
        $sha.Dispose()
    }
}

function Invoke-Cargo {
    param([Parameter(ValueFromRemainingArguments = $true)][string[]]$CommandArguments)
    & $cargoCommand @CommandArguments
    if ($LASTEXITCODE -ne 0) {
        throw "cargo command failed: cargo $($CommandArguments -join ' ')"
    }
}

if (-not (Test-Path -LiteralPath $contract -PathType Leaf)) { throw "missing $contract" }
if (-not (Test-Path -LiteralPath $manifest -PathType Leaf)) { throw "missing $manifest" }
Assert-Literal $contract 'schema = "dol-perf-contract/v1"'
Assert-Literal $contract 'contract = "closure-v1"'
Assert-Literal $contract 'evidence_schema = "dol-perf/v1"'
Assert-Literal $contract 'state = "measurement-ready"'
Assert-Literal $contract 'baseline_commit = "772f73a553f5806a365e29b799aa18a93bc0e515"'
Assert-Literal $contract 'candidate_commit = "cfbc97f446826875bb13388ad4c0206bc3665c3b"'
Assert-Literal $contract 'bootstrap_resamples = 100000'
Assert-Literal $contract 'retry_complete_pair_once = true'

foreach ($line in Get-Content -LiteralPath $manifest) {
    if ([string]::IsNullOrWhiteSpace($line)) { continue }
    if ($line -notmatch '^([0-9a-f]{64})  (.+)$') { throw "invalid fixture manifest row: $line" }
    $expected = $Matches[1]
    $relative = $Matches[2].Replace('/', [System.IO.Path]::DirectorySeparatorChar)
    $path = Join-Path $fixtureRoot $relative
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "missing fixture $path" }
    $actual = Get-NormalizedSha256 $path
    if ($actual -ne $expected) { throw "fixture hash mismatch: $relative" }
}

$contractText = Get-Content -Raw -LiteralPath $contract
if ($contractText -notmatch '(?m)^fixture_manifest_sha256 = "([0-9a-f]{64})"$') {
    throw 'contract has no fixture_manifest_sha256'
}
$expectedManifest = $Matches[1]
$actualManifest = Get-NormalizedSha256 $manifest
if ($actualManifest -ne $expectedManifest) { throw 'fixture manifest hash mismatch' }

Invoke-Cargo fmt --all --check
Invoke-Cargo check --workspace --all-targets --all-features
Invoke-Cargo clippy --workspace --all-targets --all-features -- -D warnings
Invoke-Cargo test --workspace --all-features
Invoke-Cargo test --package dol-bench
Invoke-Cargo test --package xtask
Invoke-Cargo xtask bench-check
Invoke-Cargo test --package dol-bench --test performance_milestone
Invoke-Cargo test --package dol-wire --test wire
Invoke-Cargo test --package dol-core performance_layout_contract
$aaRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("dol-perf-a-a-verify-$PID-$([guid]::NewGuid().ToString('N'))")
try {
    Invoke-Cargo xtask perf-a-a-smoke -- --artifact-dir (Join-Path $aaRoot 'artifacts')
}
finally {
    if (Test-Path -LiteralPath $aaRoot) {
        Remove-Item -LiteralPath $aaRoot -Recurse -Force
    }
}

Write-Host 'performance contract verification passed (measurement-ready; not performance-closed)'
