<#
.SYNOPSIS
  One command: capture every tape scenario unattended, then score them.

.DESCRIPTION
  Run from the repository root:
      powershell -File tools\rb_tape_bot\run_batch.ps1 [-Repeat 2] [-Scenarios a,b]
  Needs BakkesMod running with the capture plugin (1.4+) set to load at game
  start. Writes replays\batch_<timestamp>\ with the captures and results.md.
  Use -ScoreOnly <dir> to re-score an existing batch directory.
#>
param(
    [int]$Repeat = 2,
    [string[]]$Scenarios = @(),
    [string]$ScoreOnly = ""
)

$ErrorActionPreference = "Stop"
# `powershell -File ... -Scenarios a,b` delivers "a,b" as one string.
$Scenarios = @($Scenarios | ForEach-Object { $_ -split "," } | Where-Object { $_ })
$root = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$bot = Join-Path $root "tools\rb_tape_bot"
$verify = Join-Path $root "target\release\rb-verify.exe"

if ($ScoreOnly) {
    $out = (Resolve-Path $ScoreOnly).Path
} else {
    Push-Location $bot
    try {
        cargo build --release --color never
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
        $stamp = Get-Date -Format "yyyyMMdd-HHmmss"
        $out = Join-Path $root "replays\batch_$stamp"
        & (Join-Path $bot "target\release\rb_run_tapes.exe") --out $out --repeat $Repeat @Scenarios
        if ($LASTEXITCODE -ne 0) { throw "rb_run_tapes failed (exit $LASTEXITCODE); captures so far are in $out" }
    } finally {
        Pop-Location
    }
}

if (-not (Test-Path $verify)) {
    Push-Location $root
    cargo build --release -p rb_verify_cli --color never
    Pop-Location
}

# Frame count and largest gap between consecutive timestamps of a capture.
function Get-CaptureStats([string]$path) {
    $count = 0; $last = $null; $gap = 0.0
    foreach ($line in [System.IO.File]::ReadLines($path)) {
        $m = [regex]::Match($line, '"timestamp_secs":\s*(-?[0-9.eE+-]+)')
        if (-not $m.Success) { continue }
        $t = [double]::Parse($m.Groups[1].Value, [Globalization.CultureInfo]::InvariantCulture)
        if ($null -ne $last -and ($t - $last) -gt $gap) { $gap = $t - $last }
        $last = $t; $count++
    }
    [pscustomobject]@{ Frames = $count; Gap = $gap }
}

function Get-RunScore([string]$scenario, [string]$capture) {
    $text = (& $verify --scenario $scenario --against $capture 2>&1) -join "`n"
    $lag = [regex]::Match($text, 'lag (\d+) ticks').Groups[1].Value
    $err = [regex]::Match($text, 'position error: mean ([0-9.]+) uu, max ([0-9.]+) uu; first over 10 uu: (never|tick \d+); first over 100 uu: (never|tick \d+)')
    # Distance of the capture frame rb-verify aligned as the start from the
    # scenario's start location (acceptance: under 5 uu).
    $tick0 = "n/a"
    $startRow = [regex]::Match($text, 'capture frame (\d+)')
    if ($startRow.Success) {
        $loc = (Get-Content $scenario -Raw | ConvertFrom-Json).car.location
        $row = [System.IO.File]::ReadLines($capture) | Select-Object -Skip ([int]$startRow.Groups[1].Value) -First 1 | ConvertFrom-Json
        $p = $row.cars[0].position
        $tick0 = "{0:N1}" -f [math]::Sqrt([math]::Pow($p.x - $loc[0], 2) + [math]::Pow($p.y - $loc[1], 2) + [math]::Pow($p.z - $loc[2], 2))
    }
    $stats = Get-CaptureStats $capture
    if (-not $err.Success) {
        return [pscustomobject]@{ Ok = $false; Note = ($text -split "`n" | Select-Object -First 3) -join " | " }
    }
    [pscustomobject]@{
        Ok = $true; Lag = $lag; Mean = [double]$err.Groups[1].Value; Max = [double]$err.Groups[2].Value
        Over10 = $err.Groups[3].Value -replace '^tick ', ''; Over100 = $err.Groups[4].Value -replace '^tick ', ''
        Start = $tick0; Frames = $stats.Frames; Gap = $stats.Gap
    }
}

$scenarioDir = Join-Path $bot "scenarios"
$experimentDir = Join-Path $bot "experiments"
$names = Get-ChildItem $out -Filter "*_run1.jsonl" | ForEach-Object { $_.Name -replace '_run1\.jsonl$', '' } | Sort-Object

$rows = @()
foreach ($name in $names) {
    $runs = @()
    foreach ($capture in (Get-ChildItem $out -Filter "${name}_run*.jsonl" | Sort-Object Name)) {
        $runs += Get-RunScore $(if (Test-Path (Join-Path $scenarioDir "$name.json")) { Join-Path $scenarioDir "$name.json" } else { Join-Path $experimentDir "$name.json" }) $capture.FullName
    }
    $good = @($runs | Where-Object { $_.Ok })
    if ($good.Count -eq 0) { $rows += "| $name | scoring failed: $($runs[0].Note) |||||||"; continue }
    $join = { param($f) ($good | ForEach-Object { $_.$f }) -join "; " }
    $repeatable = "n/a (1 run)"
    if ($good.Count -ge 2) {
        # Spread over all runs, not just the first two.
        $means = $good | ForEach-Object { $_.Mean }
        $maxes = $good | ForEach-Object { $_.Max }
        $dMean = ($means | Measure-Object -Maximum).Maximum - ($means | Measure-Object -Minimum).Minimum
        $dMax = ($maxes | Measure-Object -Maximum).Maximum - ($maxes | Measure-Object -Minimum).Minimum
        # Session 1's floor was 2.8 uu mean / 13.8 uu max; allow a little over it.
        $verdict = if ($dMean -le 5 -and $dMax -le 20) { "yes" } else { "NO" }
        $repeatable = "{0} (spread of {1} runs: mean {2:N1}, max {3:N1})" -f $verdict, $good.Count, $dMean, $dMax
    }
    $gap = "{0:N3}" -f (($good | Measure-Object Gap -Maximum).Maximum)
    $rows += "| $name | $(& $join 'Lag') | $(($good | ForEach-Object { '{0:N1} / {1:N1}' -f $_.Mean, $_.Max }) -join '; ') | $(($good | ForEach-Object { '{0} / {1}' -f $_.Over10, $_.Over100 }) -join '; ') | $(& $join 'Frames') | $gap | $(& $join 'Start') | $repeatable |"
}

$md = @(
    "# Tape batch $(Split-Path $out -Leaf)",
    "",
    "Port (``rb-verify --scenario ... --against``) against each capture. Per-run values are separated by `;`, run 1 first; mean / max pairs use `/`.",
    "",
    "| Scenario | Lag (ticks) | Pos. error mean / max (uu) per run | First tick over 10 / 100 uu | Frames | Largest gap (s) | Start-frame error (uu) | Repeatable |",
    "|---|---|---|---|---|---|---|---|"
) + $rows + @(
    "",
    "Start-frame error is the distance of the capture's first aligned frame from the scenario's start location (acceptance: under 5 uu). ``Repeatable`` compares the two runs' error against the port, a proxy for run-to-run noise (session 1 floor: 2.8 uu mean, 13.8 uu max). Every capture is known to have one 5-tick hole (about 0.042 s, RB-RESEARCH-O009); it shows in the largest gap, it is not hidden."
)
$md | Set-Content -Encoding utf8 (Join-Path $out "results.md")
Write-Host "wrote $(Join-Path $out 'results.md')"
