# Pure orchestration helpers: safe to dot-source in headless tests. No desktop APIs.
function New-RlOwnership { ,(New-Object 'System.Collections.Generic.List[object]') }
function Add-RlOwnedProcess($Owned, $Process) {
  # Pin the native handle now; cleanup must never resolve a possibly reused PID by name.
  $null = $Process.Handle
  $Owned.Add($Process)
  return $Process
}
function Stop-RlOwnedProcesses($Owned) {
  $errors = @()
  for ($i = $Owned.Count - 1; $i -ge 0; $i--) {
    try {
      if (-not $Owned[$i].HasExited) {
        $Owned[$i].Kill()
        if (-not $Owned[$i].WaitForExit(10000)) { throw 'owned process did not exit' }
      }
    }
    catch { $errors += 'could not stop an owned process' }
  }
  if ($errors.Count) { throw ($errors -join '; ') }
}
function Assert-RlPreflight([string]$Bin, [string]$GameDir) {
  foreach ($name in 'rb_rl_bridge', 'rb_chaser_bot', 'rb_rl_game_probe', 'rb_rl_demo_count') {
    if (-not (Test-Path (Join-Path $Bin "$name.exe"))) { throw "missing required binary: $name" }
  }
  if (-not (Test-Path (Join-Path $GameDir 'RocketLeague.exe'))) { throw 'missing RocketLeague.exe' }
  if (Get-Process -Name RocketLeague, rb_rl_bridge, rb_chaser_bot -ErrorAction SilentlyContinue) {
    throw 'matching processes already running; close them manually before starting a live test'
  }
}
function Get-RlOwnedBootstrap($Launcher, [string]$GamePath) {
  # A new name/PID alone proves no ownership. Epic can delegate to an existing launcher;
  # in that case refuse to kill the game. Only a direct child of our still-live launcher
  # with the expected image is eligible. Retain its native handle before validating it.
  if ($Launcher.HasExited) { throw 'Epic launch ownership unavailable; no game process will be stopped' }
  $candidates = @(Get-CimInstance Win32_Process -Filter "Name='RocketLeague.exe'" |
    Where-Object { $_.ParentProcessId -eq $Launcher.Id -and $_.ExecutablePath -eq $GamePath })
  if ($candidates.Count -ne 1) { throw 'Epic launch ownership unavailable; no game process will be stopped' }
  $game = Get-Process -Id $candidates[0].ProcessId -ErrorAction Stop
  $null = $game.Handle
  if ($game.HasExited -or $game.StartTime -lt $Launcher.StartTime -or $Launcher.HasExited -or
      $game.Path -ne $GamePath -or $game.StartTime -ne $candidates[0].CreationDate) {
    throw 'Epic launch identity changed; no game process will be stopped'
  }
  return $game
}
function Scrub([string]$s) {
  # Rust Debug strings use backslash escapes (including escaped quotes and Unicode).
  # Consume the entire quoted token before exact bot-name comparison; prefix collisions
  # and every other human name are anonymized. A terminal unpaired backslash is
  # consumed too: malformed/unterminated tokens must fail closed to end of input.
  [regex]::Replace($s, '(name |life \d+ )"((?:[^"\\]|\\(?:.|\z))*)(?:"|\z)', {
    param($match)
    $label = switch -CaseSensitive ($match.Groups[2].Value) {
      'rusty_bullet/chaser_bot' { 'bot_a' }
      'rusty_bullet/chaser_bot_b' { 'bot_b' }
      default { 'human' }
    }
    $match.Groups[1].Value + '"' + $label + '"'
  })
}
function Invoke-RlAnalyzer([string]$Exe, [string[]]$Arguments, [string]$Output) {
  try {
    # Explicitly test LASTEXITCODE even with native error preferences disabled/default.
    $PSNativeCommandUseErrorActionPreference = $false
    & $Exe @Arguments *> $Output
    $code = $LASTEXITCODE
    if ($code -ne 0) { return "FAILED (exit $code)" }
    return 'passed'
  } catch { return 'FAILED (could not execute analyzer)' }
}
function Invoke-RlAnalyses([string]$Bin, [string]$Dir, [string]$Stamp) {
  $status = [ordered]@{ analyze = 'skipped (missing or empty inbound recording)'; census = 'skipped (missing or empty recordings)'; demolitions = 'skipped (missing or empty inbound recording)' }
  $inFile = Join-Path $Dir "$Stamp-in.bin"
  $outFile = Join-Path $Dir "$Stamp-out.bin"
  $hasIn = (Test-Path $inFile) -and (Get-Item $inFile).Length -gt 0
  $hasOut = (Test-Path $outFile) -and (Get-Item $outFile).Length -gt 0
  if ($hasIn) {
    $status.analyze = Invoke-RlAnalyzer (Join-Path $Bin 'rb_rl_game_probe.exe') @('--analyze', $inFile) (Join-Path $Dir 'analyze.raw')
    $status.demolitions = Invoke-RlAnalyzer (Join-Path $Bin 'rb_rl_demo_count.exe') @($inFile) (Join-Path $Dir 'demolitions.raw')
    if ($hasOut) { $status.census = Invoke-RlAnalyzer (Join-Path $Bin 'rb_rl_game_probe.exe') @('--census-in', $inFile, '--census-out', $outFile) (Join-Path $Dir 'census.raw') }
  }
  return $status
}
function New-RlWatcherStartInfo([string]$ScriptPath, [string]$Dir) {
  $info = [Diagnostics.ProcessStartInfo]::new('pwsh')
  $info.UseShellExecute = $false
  $info.CreateNoWindow = $true
  foreach ($arg in @('-NoProfile', '-File', $ScriptPath, $Dir)) { $info.ArgumentList.Add($arg) }
  return $info
}
function Start-RlWatcherProcess($Info) { [Diagnostics.Process]::Start($Info) }
function Assert-RlWatcher($Process) {
  if ($null -eq $Process -or $Process.HasExited) { throw 'timestamp watcher exited; timeline is incomplete' }
}
function Start-RlWatcher([string]$Dir, $Owned) {
  $watchScript = Join-Path $Dir 'rl-watch.ps1'
  $ready = Join-Path $Dir 'watcher.ready'
  Remove-Item $ready -ErrorAction SilentlyContinue
  Set-Content $watchScript @'
param($d)
$ErrorActionPreference = 'Stop'
$seen = @(Get-Content "$d\bridge.out.log" -ErrorAction SilentlyContinue).Count
New-Item -ItemType File -Path "$d\timed.log" -Force | Out-Null
Set-Content "$d\watcher.ready" 'ready'
while ($true) {
  $l = @(Get-Content "$d\bridge.out.log" -ErrorAction SilentlyContinue)
  if ($l.Count -gt $seen) {
    $l[$seen..($l.Count - 1)] | ForEach-Object { '{0:HH:mm:ss.fff} {1}' -f (Get-Date), $_ } | Add-Content "$d\timed.log"
    $seen = $l.Count
  }
  Start-Sleep -Milliseconds 200
}
'@
  $process = Add-RlOwnedProcess $Owned (Start-RlWatcherProcess (New-RlWatcherStartInfo $watchScript $Dir))
  for ($i = 0; $i -lt 50; $i++) {
    Assert-RlWatcher $process
    if (Test-Path $ready) { return $process }
    Start-Sleep -Milliseconds 100
  }
  throw 'timestamp watcher did not signal readiness'
}
