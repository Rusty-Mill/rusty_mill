#!/usr/bin/env pwsh
#
# Calibrate the decision-discipline rubric end-to-end, against both ground truths.
# PowerShell port of calibrate_scoring.sh (same two stages, native on Windows).
#
#   1. calibrate            — refit metric curves to the corpus + fit within-role
#                             weights to rank, writing assets/corpus/fitted_config.json.
#   2. reconcile --promote  — correlate each metric with per-touch DeltaV and
#                             graduate any experimental candidate that clears
#                             -MinRho and points the right way (writes back).
#
# Needs the ranked-2v2 corpus on disk (fetched if $env:BC_TOKEN is set) and a
# current assets/corpus/value_model.json (run retrain_value_model.ps1 first if the
# value features changed).
#
# Usage (from anywhere; the script cd's to the repo root):
#   $env:BC_TOKEN = "<token>"; ./scripts/calibrate_scoring.ps1     # fetch + calibrate + promote
#   ./scripts/calibrate_scoring.ps1                                # use present replays
#   ./scripts/calibrate_scoring.ps1 -MinRho 0.15                   # stricter promotion
#   ./scripts/calibrate_scoring.ps1 -Manifest path\to\manifest.json
#
# If script execution is blocked, run via:
#   pwsh -ExecutionPolicy Bypass -File scripts/calibrate_scoring.ps1
param(
    [string]$Manifest = "assets/corpus/manifest.json",
    [double]$MinRho = 0.10
)

$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')

$CorpusDir = Split-Path -Parent $Manifest
$MinReplays = 10
# Invariant formatting so a comma-decimal locale doesn't mangle the CLI arg.
$MinRhoArg = $MinRho.ToString([System.Globalization.CultureInfo]::InvariantCulture)

$present = @(Get-ChildItem -Path $CorpusDir -Filter *.replay -File -ErrorAction SilentlyContinue).Count
Write-Host "corpus replays present: $present (in $CorpusDir)"

if ($present -lt $MinReplays) {
    if ($env:BC_TOKEN) {
        Write-Host "Fetching corpus replays from the ballchasing API (gitignored)..."
        python (Join-Path $CorpusDir 'refresh_corpus_replays.py')
        if ($LASTEXITCODE -ne 0) { throw "refresh_corpus_replays.py failed ($LASTEXITCODE)" }
    }
    else {
        Write-Error ("only $present corpus replay(s) on disk and BC_TOKEN is unset.`n" +
            "  python $CorpusDir/refresh_corpus_replays.py")
        exit 1
    }
}

if (-not (Test-Path (Join-Path $CorpusDir 'value_model.json'))) {
    Write-Error ("$CorpusDir/value_model.json missing - promotion needs the value model.`n" +
        "  run scripts/retrain_value_model.ps1 first")
    exit 1
}

Write-Host ""
Write-Host "[1/2] calibrate: refit curves + fit weights to rank -> fitted_config.json"
cargo run --release -q -p replay-scoring --bin calibrate -- $Manifest
if ($LASTEXITCODE -ne 0) { throw "calibrate failed ($LASTEXITCODE)" }

Write-Host ""
Write-Host "[2/2] reconcile --promote: graduate DeltaV-vindicated candidates (min-rho $MinRhoArg)"
cargo run --release -q -p replay-scoring --bin reconcile -- $Manifest --promote --min-rho $MinRhoArg
if ($LASTEXITCODE -ne 0) { throw "reconcile failed ($LASTEXITCODE)" }

Write-Host ""
Write-Host "Done. Review the promotion verdicts above, then commit the fitted config:"
Write-Host "  git add $CorpusDir/fitted_config.json"
Write-Host "  git commit -m 'scoring: recalibrate rubric + promote DeltaV-vindicated candidates'"
