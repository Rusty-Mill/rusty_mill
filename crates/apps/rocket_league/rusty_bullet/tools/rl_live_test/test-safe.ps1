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
$sessionTokens = $null; $sessionErrors = $null
$libraryAst = [Management.Automation.Language.Parser]::ParseFile((Join-Path $PSScriptRoot 'rl-lib.ps1'), [ref]$sessionTokens, [ref]$sessionErrors)
$session = $libraryAst.Find({ param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Start-RlSession' }, $true)
$gates = @($session.FindAll({ param($node) $node -is [Management.Automation.Language.BinaryExpressionAst] -and $node.Operator -eq 'Imatch' }, $true))
Assert (@($gates | Where-Object { $_.Right.Extent.Text -eq "'waiting to speak'" }).Count -eq 1) 'session must wait for a dial'
Assert (@($gates | Where-Object { $_.Right.Extent.Text -eq "'serving bots on'" }).Count -eq 0) 'bound bot port is not game readiness'
foreach ($name in @('plain', 'quote\"private suffix', 'back\\slash', 'rusty_bullet/human', 'rusty_bullet/chaser_bot_extra', 'rusty_bullet/chaser_bot_b\"suffix', 'unicode\u{7f}', 'trailing\\')) {
  Assert ((Scrub ('PlayerSpawn name "' + $name + '" team 0')) -ceq 'PlayerSpawn name "human" team 0') "name leaked: $name"
  Assert ((Scrub ('life 12 "' + $name + '" ended')) -ceq 'life 12 "human" ended') "life leaked: $name"
}
Assert ((Scrub 'name "rusty_bullet/chaser_bot" name "rusty_bullet/chaser_bot_b"') -ceq 'name "bot_a" name "bot_b"') 'exact bot labels'
Assert ((Scrub 'name "unfinished') -ceq 'name "human"') 'unterminated name'
foreach ($prefix in @('name ', 'life 12 ')) {
  foreach ($name in @('\', 'unfinished\', 'rusty_bullet/chaser_bot\', 'escaped\"suffix\')) {
    Assert ((Scrub ($prefix + '"' + $name)) -ceq ($prefix + '"human"')) 'terminal unpaired backslash leaked'
  }
}
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
  # A verified direct child can be adopted; delegated Epic games cannot.
  # Identity checks still reject each mismatch before adoption.
  $launchedAt = [datetime]'2026-10-25T12:00:00'
  $gamePath = 'C:\game\RocketLeague.exe'
  $command = '"C:\game\RocketLeague.exe" -AUTH_LOGIN=unused -AUTH_PASSWORD=synthetic -epicapp=Sugar'
  function Reset-Bootstrap {
    $script:init = "[0005.12] Init: Command line: $command"
    $script:cimGame = [pscustomobject]@{ ProcessId = 88; ParentProcessId = 55; ExecutablePath = $gamePath; CreationDate = $launchedAt.AddSeconds(5); CommandLine = $command }
    $script:cimEpic = [pscustomobject]@{ ProcessId = 77; Name = 'EpicGamesLauncher.exe'; CreationDate = $launchedAt.AddHours(-1) }
    $script:cimGames = @($script:cimGame)
    $script:launcher = FakeProcess 55
    $script:launcher | Add-Member NoteProperty StartTime $launchedAt
    $script:game = FakeProcess 88
    $script:game | Add-Member NoteProperty Path $gamePath
    $script:game | Add-Member NoteProperty StartTime $launchedAt.AddSeconds(5).AddTicks(7)
  }
  function Get-CimInstance($ClassName, $Filter) {
    if ($Filter -eq "Name='RocketLeague.exe'") { return $script:cimGames }
    @($script:cimEpic, $script:cimGame) | Where-Object { $Filter -eq "ProcessId=$($_.ProcessId)" }
  }
  function Get-Process([int]$Id) { if ($Id -eq $script:game.Id) { return $script:game }; throw 'no such process' }
  Reset-Bootstrap
  Assert ((Get-RlOwnedBootstrap $launcher $gamePath $init) -eq $game) 'direct child rejected'
  $mismatches = @(
    { $script:cimGames = @() },
    { $script:cimGames = @($cimGame, $cimGame) },
    { $cimGame.ParentProcessId = 66 },
    { $launcher.HasExited = $true },
    { $cimGame.ExecutablePath = 'C:\other\RocketLeague.exe' },
    { $cimGame.CreationDate = $launchedAt.AddSeconds(-1); $game.StartTime = $cimGame.CreationDate },
    { $cimGame.CommandLine = "$command -extra" },
    { $cimGame.CommandLine = $command.ToLowerInvariant() },
    { $script:init = 'Init: Command line: '; $cimGame.CommandLine = '' },
    { $game.HasExited = $true },
    { $game.Path = 'C:\other\RocketLeague.exe' },
    { $game.StartTime = $launchedAt.AddSeconds(6) }
  )
  foreach ($mutate in $mismatches) {
    Reset-Bootstrap
    & $mutate
    Throws { Get-RlOwnedBootstrap $launcher $gamePath $init }
    Assert ($game.Kills -eq 0 -and $unrelated.Kills -eq 0) "refused bootstrap was killed: $mutate"
  }
  $refusal = try { Reset-Bootstrap; $cimGame.CommandLine = 'x'; Get-RlOwnedBootstrap $launcher $gamePath $init } catch { "$_" }
  Assert ($refusal -notmatch 'AUTH|synthetic') 'refusal leaked the command line'

  # Execute only the production session function AST, with every external operation mocked.
  # An independent Epic launch appears during polling and satisfies all #1046 checks:
  # Epic parent older than game, expected image, recent creation, matching shared Init line.
  . ([scriptblock]::Create($session.Extent.Text))
  foreach ($launchExited in @($false, $true)) {
    Reset-Bootstrap
    $cimGame.ParentProcessId = $cimEpic.ProcessId
    $launcher.HasExited = $launchExited
    $script:bridge = FakeProcess 44
    $script:polls = 0
    $script:launches = 0
    function Get-InitLine { if ($script:polls -gt 0) { return $script:init }; return $null }
    function Start-Sleep { $script:polls++ }
    function Start-Process {
      $script:launches++
      switch ($script:launches) {
        1 { return $script:bridge }
        2 { return $script:launcher }
        default { throw 'unexpected game relaunch' }
      }
    }
    $script:Bin = $dir; $script:GameDir = $dir; $script:EpicUrl = 'synthetic-url'
    $cimGame.ExecutablePath = Join-Path $dir 'RocketLeague.exe'
    $game.Path = $cimGame.ExecutablePath
    $sessionOwned = New-RlOwnership
    $refusal = try { Start-RlSession $dir 'test' @() $sessionOwned } catch { "$_" }
    Assert ($sessionOwned.Count -eq 2 -and -not $sessionOwned.Contains($game)) 'independent Epic game was adopted'
    Assert ($game.Kills -eq 0) 'independent Epic game was killed before cleanup'
    Assert ($refusal -like '*close the bootstrap game manually*') 'missing explicit operator cleanup instruction'
    Assert ($refusal -notmatch 'AUTH|synthetic') 'session refusal leaked the command line'
    Assert ($polls -eq 1 -and $launches -eq 2) 'independent launch must appear during wait, without relaunch'
    Stop-RlOwnedProcesses $sessionOwned
    Assert ($game.Kills -eq 0 -and $unrelated.Kills -eq 0) 'independent Epic game was killed'
    Assert ($bridge.Kills -eq 1) 'owned bridge was not cleaned up after refusal'
  }

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
    if ($Arguments[0] -eq '--analyze') {
      Assert ($Arguments.Count -eq 3 -and $Arguments[2] -ceq '--no-names') 'analysis must suppress names at the producer'
      # Model #653's output, whose optional quoted name has no legacy name prefix.
      $line = if ($Arguments -contains '--no-names') { 'PlayerSpawn: actor 38 command_id None team 0' } else { 'PlayerSpawn: actor 38 command_id None "SYNTHETIC_PRIVATE_NAME" team 0' }
      Set-Content $Output $line
      return 'FAILED (exit 9)'
    }
    return 'passed'
  }
  $status = Invoke-RlAnalyses $dir $dir 'test'
  Assert ($status.analyze -eq 'FAILED (exit 9)' -and $status.demolitions -eq 'passed' -and $status.census -like 'skipped*') 'per-stage failure/skipped status'
  Set-Content (Join-Path $dir 'test-out.bin') 'synthetic'
  $status = Invoke-RlAnalyses $dir $dir 'test'
  Assert ($status.census -eq 'passed' -and $calls.Count -eq 5) 'later stages did not run after failure'
  $analysisOutput = Get-Content (Join-Path $dir 'analyze.raw') | ForEach-Object { Scrub $_ }
  Assert ($analysisOutput -ceq 'PlayerSpawn: actor 38 command_id None team 0') 'failed analyzer output must remain anonymous'
} finally { Remove-Item -Recurse -Force $dir }
Write-Host 'rl_live_test safe regression tests passed (no game, desktop or watcher launched)'
