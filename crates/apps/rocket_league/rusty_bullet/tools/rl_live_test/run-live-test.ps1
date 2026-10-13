# One-command live test of rb_rl_bridge against the real Rocket League (Epic, offline Exhibition).
#
#   pwsh -File run-live-test.ps1 [-PlaySeconds 90] [-ThroughMatchEnd]
#
# Two chaser bots play each other (Blue vs Orange), the human spectates. The script launches the
# game the way RLBot does, walks the menus, spectates, pauses once, reloads the map once, then
# stops everything and writes report.txt (scrubbed: the human's name is replaced by "human",
# the bots by "bot_a"/"bot_b") next to the recordings in %TEMP%\rl-auto-<stamp>.
# Menu coordinates are fractions of the game window, calibrated at 2560x1440 (see rl-lib.ps1).
# Credentials never leave Start-RlSession's local variable. Offline play only.
param(
  [int]$PlaySeconds = 90,        # how long the bots play on each map before the next step
  [switch]$ThroughMatchEnd,      # also wait for the match to end and test the roll-over hold
  [switch]$HumanPlays,           # human joins Blue and drives (throttle + boost) instead of spectating
  [switch]$NoClickAfterRollover, # at the roll-over, wait for BroadcastResumed with NO click first
  [int]$MatchEndTimeoutSeconds = 600
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'rl-lib.ps1')

$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$dir   = Join-Path $env:TEMP "rl-auto-$stamp"
$dir | Set-Content "$env:TEMP\rl-auto-latest.txt"
New-Item -ItemType Directory -Force -Path $dir | Out-Null
$actions = Join-Path $dir 'actions.log'
function Note([string]$m) { $l = '{0:HH:mm:ss.fff} ACTION {1}' -f (Get-Date), $m; Add-Content $actions $l; Write-Host $l }

# Wait until the bridge log contains $Pattern (after $SkipLines existing lines); returns the match time or $null.
function Wait-Bridge([string]$Pattern, [int]$TimeoutSeconds, [int]$SkipLines = 0) {
  $end = (Get-Date).AddSeconds($TimeoutSeconds)
  while ((Get-Date) -lt $end) {
    $l = @(Get-Content "$dir\bridge.out.log" -ErrorAction SilentlyContinue) | Select-Object -Skip $SkipLines
    if ($l -match $Pattern) { return Get-Date }
    Start-Sleep -Milliseconds 300
  }
  return $null
}
function Count-Bridge { @(Get-Content "$dir\bridge.out.log" -ErrorAction SilentlyContinue).Count }

# True when the pixel at the window fraction is near-white (the highlighted NO button of a dialog).
function Test-BrightAt([double]$Fx, [double]$Fy) {
  $h = Get-RlWindow; $r = New-Object Win+RECT; [void][Win]::GetWindowRect($h, [ref]$r)
  $x = [int]($r.L + ($r.R - $r.L) * $Fx); $y = [int]($r.T + ($r.B - $r.T) * $Fy)
  $bmp = New-Object System.Drawing.Bitmap 1, 1
  $g = [System.Drawing.Graphics]::FromImage($bmp); $g.CopyFromScreen($x, $y, 0, 0, $bmp.Size)
  $c = $bmp.GetPixel(0, 0); $g.Dispose(); $bmp.Dispose()
  ($c.R + $c.G + $c.B) -gt 600
}

# The Choose Team choice and what the human does for $Seconds afterwards.
function Pick-Team { if ($HumanPlays) { Note 'Join Blue (human plays)'; Click-Rl 0.1285 0.257 } else { Note 'Spectate'; Click-Rl 0.128 0.403 } }
function Play-For([int]$Seconds) {
  if ($HumanPlays) { Note "human drives for ${Seconds}s"; Start-Sleep 12; Drive-Human $Seconds } else { Start-Sleep $Seconds }
}

$shot = 0
function Shot([string]$Tag) { $script:shot++; [void](Save-RlShot ('{0}\s{1:00}-{2}.png' -f $dir, $script:shot, $Tag)) }

# Splash -> main menu -> Play Offline -> Exhibition -> Demolish on contact -> Create Match -> Spectate.
function Start-ExhibitionMatch([bool]$FirstTime) {
  if ($FirstTime) {
    Note 'press Enter at splash'; Press-Key $Vk.Enter; Start-Sleep 6
    if (Test-BrightAt 0.556 0.567) { Note 'dismiss "not connected to Epic Online" with NO'; Click-Rl 0.556 0.567; Start-Sleep 2 }
    Note 'Play';          Click-Rl 0.10 0.556;    Start-Sleep 3
    Note 'Play Offline';  Click-Rl 0.4985 0.603;  Start-Sleep 3
    Note 'Exhibition';    Click-Rl 0.379 0.488;   Start-Sleep 3
    Note 'Mutator Settings'; Click-Rl 0.281 0.659; Start-Sleep 3
    Note 'Demolish dropdown'; Click-Rl 0.5125 0.508; Start-Sleep 2
    Note 'Demolish = ON CONTACT (FF)'; Click-Rl 0.50 0.652; Start-Sleep 1
    Shot 'mutators'
    Note 'Back'; Click-Rl 0.068 0.837; Start-Sleep 2
  }
  Start-Sleep 2   # let the screen transition finish before the click
  Shot 'before-create'
  # One click only: a second click would land on the Choose Team screen that follows.
  Note 'Create Match'
  Hover-Rl 0.279 0.716; Click-Rl 0.279 0.716
}

$bridge = $null
try {
  foreach ($exe in 'rb_rl_bridge', 'rb_chaser_bot', 'rb_rl_game_probe') {
    if (-not (Test-Path "$script:Bin\$exe.exe")) { throw "missing $script:Bin\$exe.exe - run: cargo build --release -p rb_rl_game_bridge -p rb_rlbot_bridge --bins" }
  }
  Note 'start bridge and launch the game (kill-before-login Epic relaunch)'
  $bridge = Start-RlSession -Dir $dir -Stamp $stamp -Players @('rusty_bullet/chaser_bot:0', 'rusty_bullet/chaser_bot_b:1')
  [void](Start-RlBot $dir 'rusty_bullet/chaser_bot' 'a')
  [void](Start-RlBot $dir 'rusty_bullet/chaser_bot_b' 'b')
  [void](Start-RlWatcher $dir)
  Start-Sleep 8
  Focus-Rl

  Start-ExhibitionMatch $true
  # MatchStarted has come both before and after the human picks a team, so give it a short
  # window, spectate either way, then insist on it.
  $early = Wait-Bridge 'MatchStarted' 12
  Note ('MatchStarted before choosing a team: ' + [bool]$early)
  Shot 'choose-team'
  Pick-Team
  if (-not $early -and -not (Wait-Bridge 'MatchStarted' 45)) { throw 'MatchStarted never appeared, even after choosing a team' }
  Play-For $PlaySeconds; Shot 'playing-1'

  # Pause: in offline Exhibition the pause menu does not stop the match, so no hold is expected.
  Note 'pause (Esc)'; Press-Key $Vk.Esc; Start-Sleep 5; Shot 'pause-menu'
  Note 'resume'; Click-Rl 0.50 0.383; Start-Sleep 5

  # Map reload: leave the match, create it again, pick Spectate when the new map shows Choose Team.
  $mark = Count-Bridge
  Note 'open pause menu to leave'; Press-Key $Vk.Esc; Start-Sleep 2
  Note 'Leave Match'; Click-Rl 0.50 0.658; Start-Sleep 2
  Note 'confirm Yes'; Click-Rl 0.443 0.567; Start-Sleep 8
  Start-ExhibitionMatch $false
  Start-Sleep 15; Shot 'reload-choose-team'
  Pick-Team
  Play-For $PlaySeconds; Shot 'playing-2'

  if ($ThroughMatchEnd) {
    Note 'wait for the match to end and roll into a new match'
    # The human keeps driving while the clock runs so contact (and demolitions) keep happening.
    $mark = Count-Bridge; $end = (Get-Date).AddSeconds($MatchEndTimeoutSeconds); $held = $null
    while (-not $held -and (Get-Date) -lt $end) {
      if ($HumanPlays) { Drive-Human 20 } else { Start-Sleep 5 }
      $held = Wait-Bridge 'BroadcastHeld' 1 $mark
    }
    if (-not $held) { Note 'match did not end within the timeout' }
    else {
      Note "bridge held the broadcast at roll-over: $($held.ToString('HH:mm:ss.fff'))"
      if ($NoClickAfterRollover) {
        $resumed = Wait-Bridge 'BroadcastResumed' 60 $mark
        Note ('resumed with NO click within 60s: ' + [bool]$resumed)
      }
      Shot 'rollover-choose-team'
      Pick-Team; Start-Sleep 10
    }
  }
}
catch {
  Note "ABORTED: $($_.Exception.Message)"; $failure = $_.Exception.Message
  try { Shot 'abort' } catch { Note "no abort screenshot: $($_.Exception.Message)" }
}
finally {
  Note 'stop bots, game, bridge'
  Stop-RlAll
  Get-CimInstance Win32_Process -Filter "Name='pwsh.exe'" | Where-Object { $_.CommandLine -match 'rl-watch\.ps1' } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force }
}

# ---- analysis and scrubbed report ----
$in  = Get-ChildItem $dir -Filter "$stamp-in.bin"  -ErrorAction SilentlyContinue
$out = Get-ChildItem $dir -Filter "$stamp-out.bin" -ErrorAction SilentlyContinue
if ($in -and $in.Length -gt 0) {
  Note 'analyse recordings (slow on big files)'
  & "$script:Bin\rb_rl_game_probe.exe" --analyze $in.FullName --no-names *> "$dir\analyze.raw"
  & "$script:Bin\rb_rl_game_probe.exe" --census-in $in.FullName --census-out $out.FullName *> "$dir\census.raw"
  # Demolition episodes per actor (names already labelled human/bot_a/bot_b by the tool).
  $demo = Join-Path $script:Bin 'rb_rl_demo_count.exe'
  if (Test-Path $demo) { & $demo $in.FullName *> "$dir\demolitions.raw" }
}
# The analysis is made with --no-names, so no display name is in it to begin with (the probe
# prints actor ids and teams). Scrub is a second line of defence and keeps the bots readable:
# any PlayerSpawn/life name that is not one of our bots is the human.
function Scrub([string]$s) {
  $s = $s -replace 'name "(?!rusty_bullet/)[^"]*"', 'name "human"'
  $s = $s -replace '(life \d+ )"(?!rusty_bullet/)[^"]*"', '$1"human"'
  $s = $s -replace 'rusty_bullet/chaser_bot_b', 'bot_b' -replace 'rusty_bullet/chaser_bot', 'bot_a'
  $s
}
$report = New-Object System.Collections.Generic.List[string]
$report.Add("rl live test $stamp   commit: $(git -C $script:Repo rev-parse --short HEAD)   branch: $(git -C $script:Repo branch --show-current)")
if ($failure) { $report.Add("RUN ABORTED: $failure") }
$report.Add('')
$report.Add('== timeline (actions and bridge events) ==')
$tl = @(Get-Content $actions -ErrorAction SilentlyContinue) + @(Get-Content "$dir\timed.log" -ErrorAction SilentlyContinue | ForEach-Object { $_ -replace '^(\S+) ', '$1 BRIDGE ' })
$tl | Sort-Object { $_.Substring(0, 12) } | ForEach-Object { $report.Add((Scrub $_)) }
$report.Add('')
$report.Add('== analyze (state changes collapsed) ==')
if (Test-Path "$dir\analyze.raw") {
  Get-Content "$dir\analyze.raw" | Where-Object { $_ -notmatch '^(StateChange|BallSpawn|GoalSpawn)' } | ForEach-Object { $x = Scrub $_; if ($x.Length -gt 1200) { $x = $x.Substring(0, 1200) + ' ...' }; $report.Add($x) }
}
$report.Add('')
$report.Add('== demolitions, spawns and destroys of player actors ==')
if (Test-Path "$dir\demolitions.raw") { Get-Content "$dir\demolitions.raw" | ForEach-Object { $report.Add($_) } } else { $report.Add('(rb_rl_demo_count not built)') }
$report.Add('')
$report.Add('== census ==')
if (Test-Path "$dir\census.raw") { Get-Content "$dir\census.raw" | ForEach-Object { $report.Add($_) } }
Set-Content -Path "$dir\report.txt" -Value $report
Write-Host "report: $dir\report.txt"
Write-Host 'The recordings and screenshots contain the human player''s name: scrub before sharing.'
if ($failure) { exit 1 }
