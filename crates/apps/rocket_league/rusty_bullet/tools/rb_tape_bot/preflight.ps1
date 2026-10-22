<#
.SYNOPSIS
  Keep the capture plugin in step with the installed BakkesMod: rebuild and
  reinstall it when BakkesMod, its SDK library or the plugin source changed.

.DESCRIPTION
  A plugin DLL built against an older BakkesMod SDK can crash Rocket League
  after a game or BakkesMod update even with unchanged source (the plugin's
  README, "If Rocket League crashes"). This script makes that rebuild a check
  instead of a memory: it compares what the installed DLL was built from
  (recorded in a stamp file beside it) with what is installed now, and rebuilds,
  backs up the old DLL and installs the new one when they differ.

  What counts as a change: BakkesMod's version.txt, the SHA-256 of
  pluginsdk.lib, and the SHA-256 of the plugin's source files.

  -Check only reports; it exits with an error when the plugin is stale and
  changes nothing. Run_batch.ps1 calls this script without -Check, after it
  has started BakkesMod (which updates itself) and before the game launches.

  A running game holds the DLL open, so a rebuild needs it closed. By default
  the script refuses; -CloseGame stops RocketLeague.exe, and only when a
  rebuild is actually needed (never with -Check or when the plugin is current).
  run_batch.ps1 passes it, because RLBot core kills and relaunches the game for
  every batch anyway.

  Also warns, never fails, when the game was updated after RLBot core was
  installed or after BakkesMod last updated; both are signals to look for a
  newer release before trusting a run.
#>
param(
    [switch]$Check,
    [switch]$CloseGame,
    [int]$CloseTimeoutSec = 15,
    [string]$BakkesModDir = (Join-Path $env:APPDATA "bakkesmod\bakkesmod"),
    [string]$PluginSource = (Join-Path $PSScriptRoot "..\..\bakkesmod-plugin\rusty_bullet_capture"),
    [string]$GameDir = "C:\Program Files\Epic Games\rocketleague",
    [string]$RlbotDir = (Join-Path $env:LOCALAPPDATA "RLBot5\bin")
)

$ErrorActionPreference = "Stop"
$PluginName = "rusty_bullet_capture"

function Get-FileSha256([string]$path) {
    (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash
}

# One hash over the files that decide the build, in a fixed order.
function Get-SourceSha256([string]$dir) {
    $names = "CMakeLists.txt", "RustyBulletCapturePlugin.cpp", "RustyBulletCapturePlugin.h"
    $parts = foreach ($n in $names) {
        $p = Join-Path $dir $n
        if (-not (Test-Path -LiteralPath $p -PathType Leaf)) { throw "plugin source file missing: $p" }
        "$n=$(Get-FileSha256 $p)"
    }
    $bytes = [Text.Encoding]::UTF8.GetBytes(($parts -join "`n"))
    $sha = [Security.Cryptography.SHA256]::Create()
    try { ([BitConverter]::ToString($sha.ComputeHash($bytes))) -replace "-", "" } finally { $sha.Dispose() }
}

function Get-Current([string]$bmDir, [string]$srcDir) {
    $versionFile = Join-Path $bmDir "version.txt"
    $lib = Join-Path $bmDir "bakkesmodsdk\lib\pluginsdk.lib"
    foreach ($f in $versionFile, $lib) {
        if (-not (Test-Path -LiteralPath $f -PathType Leaf)) {
            throw "BakkesMod is not installed where expected: '$f' is missing (-BakkesModDir)"
        }
    }
    [pscustomobject]@{
        bakkesmod_version = (Get-Content -LiteralPath $versionFile -Raw).Trim()
        sdk_lib_sha256    = Get-FileSha256 $lib
        source_sha256     = Get-SourceSha256 $srcDir
    }
}

# $null when nothing was stamped; throws on a stamp that is present but unreadable,
# because treating that as "absent" would hide a corrupted install.
function Read-Stamp([string]$path) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { return $null }
    try { Get-Content -LiteralPath $path -Raw | ConvertFrom-Json -ErrorAction Stop }
    catch { throw "stamp '$path' is unreadable ($($_.Exception.Message)); delete it to force a rebuild" }
}

# Why the installed plugin is stale, as one line per reason; empty when current.
function Get-StaleReasons($current, $stamp, [bool]$dllPresent) {
    if (-not $dllPresent) { return @("no installed $PluginName.dll") }
    if ($null -eq $stamp) { return @("installed DLL has no build stamp (built by hand or before this script)") }
    $reasons = @()
    if ($stamp.bakkesmod_version -ne $current.bakkesmod_version) {
        $reasons += "BakkesMod changed: built for $($stamp.bakkesmod_version), installed $($current.bakkesmod_version)"
    }
    if ($stamp.sdk_lib_sha256 -ne $current.sdk_lib_sha256) { $reasons += "pluginsdk.lib changed" }
    if ($stamp.source_sha256 -ne $current.source_sha256) { $reasons += "plugin source changed" }
    $reasons
}

function Write-GameWarnings([string]$gameDir, [string]$rlbotDir, [string]$bmDir) {
    $game = Join-Path $gameDir "Binaries\Win64\RocketLeague.exe"
    if (-not (Test-Path -LiteralPath $game -PathType Leaf)) { return }
    $gameTime = (Get-Item -LiteralPath $game).LastWriteTime
    $core = Join-Path $rlbotDir "RLBotServer.exe"
    if ((Test-Path -LiteralPath $core -PathType Leaf) -and (Get-Item -LiteralPath $core).LastWriteTime -lt $gameTime) {
        Write-Warning ("Rocket League was updated ($gameTime) after RLBot core was installed " +
            "($((Get-Item -LiteralPath $core).LastWriteTime)). Its closed-source bridge talks to the game; " +
            "check https://github.com/RLBot/core/releases and run launcher.exe to update.")
    }
    $bmVersion = Join-Path $bmDir "version.txt"
    if ((Test-Path -LiteralPath $bmVersion -PathType Leaf) -and (Get-Item -LiteralPath $bmVersion).LastWriteTime -lt $gameTime) {
        Write-Warning ("Rocket League was updated ($gameTime) after BakkesMod last updated " +
            "($((Get-Item -LiteralPath $bmVersion).LastWriteTime)). If BakkesMod shows an update prompt, accept it, then rerun.")
    }
}

# The running game holds the plugin DLL open, so it cannot be replaced until the game exits.
# Closing it is opt-in: run by hand this must never end a game someone is playing.
function Close-GameIfRunning([bool]$allowClose, [int]$timeoutSec) {
    if (-not (Get-Process -Name "RocketLeague" -ErrorAction SilentlyContinue)) { return }
    if (-not $allowClose) {
        throw "Rocket League is running and holds the plugin DLL; close it and rerun (or pass -CloseGame)"
    }
    Write-Host "preflight: closing Rocket League to replace the plugin DLL"
    Stop-Process -Name "RocketLeague" -Force -ErrorAction SilentlyContinue
    $deadline = (Get-Date).AddSeconds($timeoutSec)
    while ((Get-Process -Name "RocketLeague" -ErrorAction SilentlyContinue) -and (Get-Date) -lt $deadline) {
        Start-Sleep -Milliseconds 200
    }
    if (Get-Process -Name "RocketLeague" -ErrorAction SilentlyContinue) {
        throw "Rocket League did not exit within $timeoutSec s; close it yourself and rerun"
    }
}

function Install-Plugin([string]$srcDir, [string]$pluginsDir, $current, $stamp, [bool]$closeGame, [int]$closeTimeoutSec) {
    # Refuse before building when the game cannot be closed, but only stop it once the build has
    # succeeded: the build does not need the game closed, so a failed build must not end a session.
    if (-not $closeGame -and (Get-Process -Name "RocketLeague" -ErrorAction SilentlyContinue)) {
        throw "Rocket League is running and holds the plugin DLL; close it and rerun (or pass -CloseGame)"
    }
    $build = Join-Path $srcDir "build"
    if (Test-Path -LiteralPath $build) { Remove-Item -LiteralPath $build -Recurse -Force }
    Push-Location $srcDir
    try {
        cmake -B build
        if ($LASTEXITCODE -ne 0) { throw "cmake configure failed (exit $LASTEXITCODE)" }
        cmake --build build --config Release
        if ($LASTEXITCODE -ne 0) { throw "cmake build failed (exit $LASTEXITCODE)" }
    } finally {
        Pop-Location
    }
    $built = Join-Path $build "Release\$PluginName.dll"
    if (-not (Test-Path -LiteralPath $built -PathType Leaf)) { throw "build succeeded but '$built' is missing" }

    Close-GameIfRunning $closeGame $closeTimeoutSec
    $installed = Join-Path $pluginsDir "$PluginName.dll"
    if (Test-Path -LiteralPath $installed -PathType Leaf) {
        $tag = if ($null -ne $stamp) { "bm$($stamp.bakkesmod_version)" } else { "unstamped" }
        Copy-Item -LiteralPath $installed (Join-Path $pluginsDir "$PluginName.$tag.dll.bak") -Force
    }
    Copy-Item -LiteralPath $built $installed -Force
    if ((Get-FileSha256 $built) -ne (Get-FileSha256 $installed)) { throw "installed DLL does not match the build" }

    $new = [ordered]@{
        bakkesmod_version = $current.bakkesmod_version
        sdk_lib_sha256    = $current.sdk_lib_sha256
        source_sha256     = $current.source_sha256
        built_utc         = (Get-Date).ToUniversalTime().ToString("o")
    }
    $new | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $pluginsDir "$PluginName.stamp.json") -Encoding UTF8
}

$PluginSource = (Resolve-Path -LiteralPath $PluginSource).Path
$pluginsDir = Join-Path $BakkesModDir "plugins"
if (-not (Test-Path -LiteralPath $pluginsDir -PathType Container)) {
    throw "BakkesMod plugins folder missing: '$pluginsDir' (-BakkesModDir)"
}
$current = Get-Current $BakkesModDir $PluginSource
$stamp = Read-Stamp (Join-Path $pluginsDir "$PluginName.stamp.json")
$dllPresent = Test-Path -LiteralPath (Join-Path $pluginsDir "$PluginName.dll") -PathType Leaf
$reasons = @(Get-StaleReasons $current $stamp $dllPresent)

Write-GameWarnings $GameDir $RlbotDir $BakkesModDir

if ($reasons.Count -eq 0) {
    Write-Host "preflight: $PluginName is current for BakkesMod $($current.bakkesmod_version)"
    return
}
foreach ($r in $reasons) { Write-Host "preflight: $r" }
if ($Check) { throw "preflight: $PluginName is stale ($($reasons -join '; ')); run preflight.ps1 without -Check to rebuild" }

Install-Plugin $PluginSource $pluginsDir $current $stamp ([bool]$CloseGame) $CloseTimeoutSec
Write-Host "preflight: rebuilt and installed $PluginName for BakkesMod $($current.bakkesmod_version)"
