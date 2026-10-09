#!/usr/bin/env pwsh
#
# Retrain the corpus value model end-to-end and report the held-out AUC.
# PowerShell port of retrain_value_model.sh (same behaviour, native on Windows).
#
#   1. ensure the gitignored ranked-2v2 corpus .replay files are on disk (fetch
#      if $env:BC_TOKEN is set and they're missing);
#   2. run train_corpus, which prints the logistic-vs-GBT VAL_AUC head-to-head and
#      writes assets/corpus/value_model.json (the shipped GBT);
#   3. remind you to commit the refreshed model.
#
# Run this after changing the value feature set so the shipped corpus model
# matches the featurizer. The per-match DeltaV path (app/viewer) needs no retrain.
#
# Usage (from anywhere; the script cd's to the repo root):
#   $env:BC_TOKEN = "<token>"; ./scripts/retrain_value_model.ps1     # fetch + train
#   ./scripts/retrain_value_model.ps1                                # train on present replays
#   ./scripts/retrain_value_model.ps1 path\to\manifest.json          # alternate corpus
#
# If script execution is blocked, run via:
#   pwsh -ExecutionPolicy Bypass -File scripts/retrain_value_model.ps1
#
# Get a token at https://ballchasing.com/upload (Account -> API key).
param([string]$Manifest = "assets/corpus/manifest.json")

$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')

$CorpusDir = Split-Path -Parent $Manifest
$MinReplays = 10

$present = @(Get-ChildItem -Path $CorpusDir -Filter *.replay -File -ErrorAction SilentlyContinue).Count
Write-Host "corpus replays present: $present (in $CorpusDir)"

if ($present -lt $MinReplays) {
    if ($env:BC_TOKEN) {
        Write-Host "Fetching corpus replays from the ballchasing API (gitignored, not committed)..."
        python (Join-Path $CorpusDir 'refresh_corpus_replays.py')
        if ($LASTEXITCODE -ne 0) { throw "refresh_corpus_replays.py failed ($LASTEXITCODE)" }
    }
    else {
        Write-Error ("only $present corpus replay(s) on disk and BC_TOKEN is unset.`n" +
            "Set a ballchasing token and re-run, or fetch manually:`n" +
            "  python $CorpusDir/refresh_corpus_replays.py")
        exit 1
    }
}

Write-Host "Training the value model on the corpus (release build; parses every replay)..."
cargo run --release -q -p replay-value --bin train_corpus -- $Manifest
if ($LASTEXITCODE -ne 0) { throw "train_corpus failed ($LASTEXITCODE)" }

Write-Host ""
Write-Host "Done. Review the VAL_AUC head-to-head above (the GBT is the shipped model)."
Write-Host "If it looks good, commit the refreshed model:"
Write-Host "  git add $CorpusDir/value_model.json"
Write-Host "  git commit -m 'value: retrain corpus model on vcfg-v2 feature set'"
