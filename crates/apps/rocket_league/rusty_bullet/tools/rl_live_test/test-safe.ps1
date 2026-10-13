# Dependency-free deterministic tests. Never load rl-lib.ps1 or launch game/desktop/watcher code.
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
. (Join-Path $PSScriptRoot 'rl-safe.ps1')
function Assert($Condition, [string]$Message) { if (-not $Condition) { throw $Message } }
function Throws([scriptblock]$Body) {
  $threw = $false
  try { & $Body } catch { $threw = $true }
  Assert $threw 'expected failure'
}
function FakeProcess([int]$Id, [bool]$Exited = $false) {
  $p = [pscustomobject]@{ Id = $Id; Handle = $Id; HasExited = $Exited; Kills = 0 }
  $p | Add-Member ScriptMethod Kill { $this.Kills++ }
  $p | Add-Member ScriptMethod WaitForExit { param($Timeout) return $true }
  return $p
}
# Parse every production script without executing its desktop entry point.
foreach ($file in Get-ChildItem $PSScriptRoot -Filter '*.ps1') {
  $tokens = $null; $errors = $null
  $null = [Management.Automation.Language.Parser]::ParseFile($file.FullName, [ref]$tokens, [ref]$errors)
  Assert ($errors.Count -eq 0) "parse failed: $($file.Name): $errors"
}
foreach ($name in @('plain', 'quote\"private suffix', 'back\\slash', 'rusty_bullet/human', 'rusty_bullet/chaser_bot_extra', 'rusty_bullet/chaser_bot_b\"suffix', 'unicode\u{7f}', 'trailing\\')) {
  Assert ((Scrub ('PlayerSpawn name "' + $name + '" team 0')) -ceq 'PlayerSpawn name "human" team 0') "name leaked: $name"
  Assert ((Scrub ('life 12 "' + $name + '" ended')) -ceq 'life 12 "human" ended') "life leaked: $name"
}
Assert ((Scrub 'name "rusty_bullet/chaser_bot" name "rusty_bullet/chaser_bot_b"') -ceq 'name "bot_a" name "bot_b"') 'exact bot labels'
Assert ((Scrub 'name "unfinished') -ceq 'name "human"') 'unterminated name'
Assert ((Scrub 'name "Rusty_bullet/chaser_bot"') -ceq 'name "human"') 'case collision'

$dir = Join-Path ([IO.Path]::GetTempPath()) ('rl safe tests ' + [guid]::NewGuid())
New-Item -ItemType Directory $dir | Out-Null
try {
  # Failed preflight, including missing binaries with matching preexisting processes,
  # cannot add ownership or invoke a destructive operation.
  $script:unrelated = FakeProcess 99
  function Get-Process { $script:unrelated }
  function Stop-Process { throw 'name/PID-based cleanup is forbidden' }
  $owned = New-RlOwnership
  Throws { Assert-RlPreflight $dir $dir }
  Stop-RlOwnedProcesses $owned
  Assert ($unrelated.Kills -eq 0) 'preflight killed an unrelated process'
  foreach ($name in 'rb_rl_bridge', 'rb_chaser_bot', 'rb_rl_game_probe', 'rb_rl_demo_count', 'RocketLeague') {
    Set-Content (Join-Path $dir "$name.exe") 'fixture'
  }
  Throws { Assert-RlPreflight $dir $dir }
  $first = FakeProcess 1; $second = FakeProcess 2; $exited = FakeProcess 3 $true
  $null = Add-RlOwnedProcess $owned $first
  $null = Add-RlOwnedProcess $owned $second
  $null = Add-RlOwnedProcess $owned $exited
  Stop-RlOwnedProcesses $owned
  Assert ($first.Kills -eq 1 -and $second.Kills -eq 1 -and $exited.Kills -eq 0 -and $unrelated.Kills -eq 0) 'owned-only cleanup'
  # Delegated Epic launches / exited launch handles are never accepted as ownership.
  Throws { Get-RlOwnedBootstrap $exited 'C:\game\RocketLeague.exe' }
  function Get-CimInstance { [pscustomobject]@{ ParentProcessId = 99; ExecutablePath = 'C:\game\RocketLeague.exe'; ProcessId = 88 } }
  Throws { Get-RlOwnedBootstrap $first 'C:\game\RocketLeague.exe' }
  Assert ($unrelated.Kills -eq 0) 'ambiguous bootstrap was killed'

  $info = New-RlWatcherStartInfo (Join-Path $dir 'watch script.ps1') $dir
  Assert ($info.ArgumentList.Count -eq 4) 'watcher argument count'
  Assert ($info.ArgumentList[2] -eq (Join-Path $dir 'watch script.ps1') -and $info.ArgumentList[3] -eq $dir) 'space-containing paths changed'
  $script:watch = FakeProcess 4
  function Start-RlWatcherProcess($Info) {
    Assert ($Info.ArgumentList[3] -eq $dir) 'wrong watcher directory'
    Set-Content (Join-Path $dir 'watcher.ready') 'ready'
    return $script:watch
  }
  $watchOwned = New-RlOwnership
  $handle = Start-RlWatcher $dir $watchOwned
  Assert ($handle -eq $watch -and $watchOwned.Count -eq 1) 'watcher handle lost'
  $watch.HasExited = $true
  Throws { Assert-RlWatcher $handle }
  Throws { Start-RlWatcher $dir $watchOwned }
  $watch.HasExited = $false
  function Start-RlWatcherProcess($Info) { $script:watch }
  function Start-Sleep {} # No real startup polling delay in the timeout test.
  Throws { Start-RlWatcher $dir $watchOwned }

  # Run the actual entry point against a stub library: preflight must emit a failed
  # report/exit without ever reaching desktop APIs, launch, or name-based cleanup.
  $sandbox = Join-Path $dir 'entry point'
  New-Item -ItemType Directory $sandbox | Out-Null
  Copy-Item (Join-Path $PSScriptRoot 'run-live-test.ps1') $sandbox
  Copy-Item (Join-Path $PSScriptRoot 'rl-safe.ps1') $sandbox
  Set-Content (Join-Path $sandbox 'rl-lib.ps1') @'
. (Join-Path $PSScriptRoot 'rl-safe.ps1')
$script:Repo = $PSScriptRoot
$script:Bin = Join-Path $PSScriptRoot 'missing binaries'
$script:GameDir = $PSScriptRoot
function Get-Process { [pscustomobject]@{ Id = 99; Name = 'RocketLeague' } }
function Start-Process { throw 'UNSAFE launch attempted' }
function Stop-Process { throw 'UNSAFE cleanup attempted' }
function Save-RlShot { throw 'UNSAFE screenshot attempted' }
function git { 'synthetic-revision' }
'@
  $oldTemp = $env:TEMP
  try {
    $env:TEMP = $sandbox
    & (Get-Command pwsh -CommandType Application).Source -NoProfile -File (Join-Path $sandbox 'run-live-test.ps1') *> (Join-Path $sandbox 'entry.log')
    Assert ($LASTEXITCODE -eq 1) 'failed preflight returned success'
    $report = Get-Content (Join-Path (Get-Content (Join-Path $sandbox 'rl-auto-latest.txt')) 'report.txt') -Raw
    Assert ($report -match 'run: FAILED' -and $report -match 'cleanup: passed' -and $report -match 'analyze: skipped') 'preflight report status'
    Assert ($report -notmatch 'UNSAFE') 'preflight reached unsafe operation'
  } finally { $env:TEMP = $oldTemp }

  # Real native child stubs, not a real analyzer or watcher. Exercise explicit exit codes.
  $pwsh = (Get-Command pwsh -CommandType Application).Source
  $raw = Join-Path $dir 'native.raw'
  Assert ((Invoke-RlAnalyzer $pwsh @('-NoProfile', '-Command', 'exit 7') $raw) -eq 'FAILED (exit 7)') 'native nonzero ignored'
  Assert ((Invoke-RlAnalyzer $pwsh @('-NoProfile', '-Command', 'exit 0') $raw) -eq 'passed') 'native zero rejected'
  Assert ((Invoke-RlAnalyzer (Join-Path $dir 'missing.exe') @() $raw) -like 'FAILED*') 'missing analyzer passed'
  $status = Invoke-RlAnalyses $dir $dir 'test'
  Assert (@($status.Values | Where-Object { $_ -like 'skipped*' }).Count -eq 3) 'missing input status'
  Set-Content (Join-Path $dir 'test-in.bin') 'synthetic'
  $script:calls = @()
  function Invoke-RlAnalyzer($Exe, $Arguments, $Output) {
    $script:calls += ,@($Exe, $Arguments, $Output)
    if ($Arguments[0] -eq '--analyze') { return 'FAILED (exit 9)' }
    return 'passed'
  }
  $status = Invoke-RlAnalyses $dir $dir 'test'
  Assert ($status.analyze -eq 'FAILED (exit 9)' -and $status.demolitions -eq 'passed' -and $status.census -like 'skipped*') 'per-stage failure/skipped status'
  Set-Content (Join-Path $dir 'test-out.bin') 'synthetic'
  $status = Invoke-RlAnalyses $dir $dir 'test'
  Assert ($status.census -eq 'passed' -and $calls.Count -eq 5) 'later stages did not run after failure'
} finally { Remove-Item -Recurse -Force $dir }
Write-Host 'rl_live_test safe regression tests passed (no game, desktop or watcher launched)'
