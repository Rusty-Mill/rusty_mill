# Helpers for the live Rocket League / rb_rl_bridge test. Dot-source this file.
# Credentials rule: the Epic login args live only in a local variable inside Start-RlGame
# and are never printed, logged or written to disk.

# Repo root = six levels above this folder (crates/apps/rocket_league/rusty_bullet/tools/rl_live_test).
$script:Repo   = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..\..\..\..')).Path
$script:Bin    = Join-Path $script:Repo 'target\release'
$script:GameDir = 'C:\Program Files\Epic Games\rocketleague\Binaries\Win64'
$script:LaunchLog = Join-Path $env:USERPROFILE 'OneDrive\Documents\My Games\Rocket League\TAGame\Logs\Launch.log'
$script:EpicUrl = 'com.epicgames.launcher://apps/9773aa1aa54f4f7b80e44bef04986cea%3A530145df28a24424923f5828cc9031a1%3ASugar?action=launch&silent=true'

Add-Type -AssemblyName System.Drawing, System.Windows.Forms
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class Win {
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int c);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, int dx, int dy, uint d, UIntPtr e);
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr e);
  [DllImport("user32.dll")] public static extern uint MapVirtualKey(uint code, uint type);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
}
'@
[void][Win]::SetProcessDPIAware()

# Polled while the game starts, and the game renames the old Launch.log to a backup and creates a
# new one at that moment. Select-String opens the file without delete-sharing, which can make that
# rename fail and kill the game before it logs anything (seen: RocketLeague.exe dying after 7 s,
# Launch.log left empty). So open it the way RLBot's own reader does and also allow delete.
function Get-InitLine {
  if (-not (Test-Path $script:LaunchLog)) { return $null }
  try {
    $share = [IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete
    $fs = [IO.File]::Open($script:LaunchLog, [IO.FileMode]::Open, [IO.FileAccess]::Read, $share)
    try {
      $sr = New-Object IO.StreamReader($fs)
      $last = $null
      while ($null -ne ($line = $sr.ReadLine())) { if ($line.Contains('Init: Command line:')) { $last = $line } }
      $last
    } finally { $fs.Dispose() }
  } catch { $null }
}

function Stop-RlAll {
  Stop-Process -Name rb_chaser_bot -Force -ErrorAction SilentlyContinue
  Stop-Process -Name RocketLeague -Force -ErrorAction SilentlyContinue
  Start-Sleep 3
  Stop-Process -Name rb_rl_bridge -Force -ErrorAction SilentlyContinue
}

# Starts the bridge (recording into $Dir), then launches the game the way RLBot does:
# silent Epic launch, kill the moment a NEW Init line appears (before login spends the
# single-use code), relaunch with -rlbot args. Returns the bridge Process.
function Start-RlSession([string]$Dir, [string]$Stamp, [string[]]$Players) {
  Stop-RlAll
  New-Item -ItemType Directory -Force -Path $Dir | Out-Null
  $before = Get-InitLine
  $bridge = Start-Process "$script:Bin\rb_rl_bridge.exe" -WorkingDirectory $Dir -PassThru -WindowStyle Hidden `
    -ArgumentList (@('--record-in', "$Stamp-in.bin", '--record-out', "$Stamp-out.bin") + $Players) `
    -RedirectStandardOutput "$Dir\bridge.out.log" -RedirectStandardError "$Dir\bridge.err.log"
  Start-Process cmd.exe -WindowStyle Hidden -ArgumentList "/c start `"`" `"$script:EpicUrl`""
  $line = $null
  for ($i = 0; $i -lt 240; $i++) {
    $cur = Get-InitLine
    if ($cur -and $cur -ne $before) { $line = $cur; break }
    Start-Sleep -Milliseconds 500
  }
  if (-not $line) { throw 'no new Init line from the Epic launch' }
  Stop-Process -Name RocketLeague -Force -ErrorAction SilentlyContinue
  while (Get-Process RocketLeague -ErrorAction SilentlyContinue) { Start-Sleep -Milliseconds 300 }
  $raw = $line -replace '^.*Init: Command line:\s*', ''
  $tokens = [regex]::Matches($raw, '(?:"[^"]*"|\S)+') | ForEach-Object Value
  $keep = $tokens | Where-Object { $_ -notmatch '^"?[A-Za-z]:\\' -and $_ -notmatch '^-rlbot$|^RLBot_|^-nomovie$' }
  $login = $keep -join ' '
  [void](Start-Process "$script:GameDir\RocketLeague.exe" -WorkingDirectory $script:GameDir `
    -ArgumentList "-rlbot RLBot_ControllerURL=127.0.0.1:23233 RLBot_PacketSendRate=240 -nomovie $login")
  $login = $null; $raw = $null; $tokens = $null; $keep = $null; $line = $null; $cur = $null; $before = $null
  for ($i = 0; $i -lt 100; $i++) {
    if ((Get-Content "$Dir\bridge.out.log" -ErrorAction SilentlyContinue) -match 'serving bots on') { return $bridge }
    Start-Sleep 3
  }
  throw 'the game did not connect to the bridge'
}

function Start-RlBot([string]$Dir, [string]$AgentId, [string]$Tag) {
  $env:RLBOT_SERVER_PORT = '23234'; $env:RLBOT_AGENT_ID = $AgentId
  Start-Process "$script:Bin\rb_chaser_bot.exe" -PassThru -WindowStyle Hidden `
    -RedirectStandardOutput "$Dir\bot-$Tag.out.log" -RedirectStandardError "$Dir\bot-$Tag.err.log"
}

function Get-RlWindow {
  $p = Get-Process RocketLeague -ErrorAction SilentlyContinue | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
  if (-not $p) { throw 'no Rocket League window' }
  $p.MainWindowHandle
}

function Focus-Rl { $h = Get-RlWindow; [void][Win]::ShowWindow($h, 9); [void][Win]::SetForegroundWindow($h); Start-Sleep -Milliseconds 400 }

# Screenshot of the game window to $Path (PNG); returns window rect.
function Save-RlShot([string]$Path) {
  $h = Get-RlWindow; $r = New-Object Win+RECT; [void][Win]::GetWindowRect($h, [ref]$r)
  $w = $r.R - $r.L; $ht = $r.B - $r.T
  $bmp = New-Object System.Drawing.Bitmap $w, $ht
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.CopyFromScreen($r.L, $r.T, 0, 0, $bmp.Size)
  $bmp.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png); $g.Dispose(); $bmp.Dispose()
  $r
}

# Click at window-relative fractions (0..1) of the game window.
function Click-Rl([double]$Fx, [double]$Fy) {
  Focus-Rl
  $h = Get-RlWindow; $r = New-Object Win+RECT; [void][Win]::GetWindowRect($h, [ref]$r)
  $x = [int]($r.L + ($r.R - $r.L) * $Fx); $y = [int]($r.T + ($r.B - $r.T) * $Fy)
  [void][Win]::SetCursorPos($x, $y); Start-Sleep -Milliseconds 250
  [Win]::mouse_event(2, 0, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 80
  [Win]::mouse_event(4, 0, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 400
}

# Move the cursor onto a spot and let the menu register the hover (some buttons ignore a click
# that arrives with no preceding hover).
function Hover-Rl([double]$Fx, [double]$Fy) {
  Focus-Rl
  $h = Get-RlWindow; $r = New-Object Win+RECT; [void][Win]::GetWindowRect($h, [ref]$r)
  $x = [int]($r.L + ($r.R - $r.L) * $Fx); $y = [int]($r.T + ($r.B - $r.T) * $Fy)
  [void][Win]::SetCursorPos($x - 6, $y - 4); Start-Sleep -Milliseconds 150
  [void][Win]::SetCursorPos($x, $y); Start-Sleep -Milliseconds 700
}

# Hold the human's throttle (W) and boost (left mouse) for $Seconds, weaving left/right so the
# car does not just pin itself against a wall. Always releases everything on the way out.
function Drive-Human([int]$Seconds) {
  Focus-Rl
  $w = 0x57; $a = 0x41; $d = 0x44
  $sw = [byte][Win]::MapVirtualKey($w, 0)
  [Win]::keybd_event($w, $sw, 0, [UIntPtr]::Zero)
  [Win]::mouse_event(2, 0, 0, 0, [UIntPtr]::Zero)
  try {
    $end = (Get-Date).AddSeconds($Seconds); $left = $true
    while ((Get-Date) -lt $end) {
      $k = if ($left) { $a } else { $d }; $sk = [byte][Win]::MapVirtualKey($k, 0)
      [Win]::keybd_event($k, $sk, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 600
      [Win]::keybd_event($k, $sk, 2, [UIntPtr]::Zero); Start-Sleep -Milliseconds 2400
      $left = -not $left
    }
  } finally {
    [Win]::mouse_event(4, 0, 0, 0, [UIntPtr]::Zero)
    [Win]::keybd_event($w, $sw, 2, [UIntPtr]::Zero)
    foreach ($k in $a, $d) { [Win]::keybd_event($k, [byte][Win]::MapVirtualKey($k, 0), 2, [UIntPtr]::Zero) }
  }
}

# Press a key by virtual-key code, with a scan code so DirectInput-style games see it.
function Press-Key([byte]$Vk, [int]$HoldMs = 80) {
  Focus-Rl
  $sc = [byte][Win]::MapVirtualKey($Vk, 0)
  [Win]::keybd_event($Vk, $sc, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds $HoldMs
  [Win]::keybd_event($Vk, $sc, 2, [UIntPtr]::Zero); Start-Sleep -Milliseconds 300
}
$script:Vk = @{ Enter = 0x0D; Esc = 0x1B; Up = 0x26; Down = 0x28; Left = 0x25; Right = 0x27; Space = 0x20 }

# Timestamped copy of NEW bridge log lines (lines already present are skipped, so nothing is
# mislabelled with the watcher's start time). Runs as a detached process: a PowerShell job dies
# with the tool call that started it.
function Start-RlWatcher([string]$Dir) {
  $script = Join-Path $env:TEMP 'rl-watch.ps1'
  Set-Content $script @'
param($d)
$seen = @(Get-Content "$d\bridge.out.log" -ErrorAction SilentlyContinue).Count
while ($true) {
  $l = @(Get-Content "$d\bridge.out.log" -ErrorAction SilentlyContinue)
  if ($l.Count -gt $seen) {
    $l[$seen..($l.Count - 1)] | ForEach-Object { '{0:HH:mm:ss.fff} {1}' -f (Get-Date), $_ } | Add-Content "$d\timed.log"
    $seen = $l.Count
  }
  Start-Sleep -Milliseconds 200
}
'@
  Start-Process pwsh -WindowStyle Hidden -PassThru -ArgumentList '-NoProfile', '-File', $script, $Dir
}
