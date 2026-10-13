# Helpers for the live Rocket League / rb_rl_bridge test. Dot-source this file.
. (Join-Path $PSScriptRoot 'rl-safe.ps1')
# Credentials rule: the Epic login args live only in a local variable inside Start-RlSession
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

# Bootstrap ownership must be verified before stopping/relaunching any game.
function Start-RlSession([string]$Dir, [string]$Stamp, [string[]]$Players, $Owned) {
  New-Item -ItemType Directory -Force -Path $Dir | Out-Null
  $before = Get-InitLine
  $bridge = Start-Process "$script:Bin\rb_rl_bridge.exe" -WorkingDirectory $Dir -PassThru -WindowStyle Hidden `
    -ArgumentList (@('--record-in', "$Stamp-in.bin", '--record-out', "$Stamp-out.bin") + $Players) `
    -RedirectStandardOutput "$Dir\bridge.out.log" -RedirectStandardError "$Dir\bridge.err.log"
  $null = Add-RlOwnedProcess $Owned $bridge
  $launcher = Add-RlOwnedProcess $Owned (Start-Process cmd.exe -PassThru -WindowStyle Hidden -ArgumentList "/c start `"`" `"$script:EpicUrl`"")
  $line = $null
  for ($i = 0; $i -lt 240; $i++) {
    $cur = Get-InitLine
    if ($cur -and $cur -ne $before) { $line = $cur; break }
    Start-Sleep -Milliseconds 500
  }
  if (-not $line) { throw 'no new Init line from the Epic launch' }
  $bootstrap = Add-RlOwnedProcess $Owned (Get-RlOwnedBootstrap $launcher (Join-Path $script:GameDir 'RocketLeague.exe'))
  $bootstrap.Kill()
  if (-not $bootstrap.WaitForExit(10000)) { throw 'owned bootstrap game did not exit' }
  $raw = $line -replace '^.*Init: Command line:\s*', ''
  $tokens = [regex]::Matches($raw, '(?:"[^"]*"|\S)+') | ForEach-Object Value
  $keep = $tokens | Where-Object { $_ -notmatch '^"?[A-Za-z]:\\' -and $_ -notmatch '^-rlbot$|^RLBot_|^-nomovie$' }
  $login = $keep -join ' '
  $script:OwnedGame = Add-RlOwnedProcess $Owned (Start-Process "$script:GameDir\RocketLeague.exe" -PassThru -WorkingDirectory $script:GameDir `
    -ArgumentList "-rlbot RLBot_ControllerURL=127.0.0.1:23233 RLBot_PacketSendRate=240 -nomovie $login")
  $login = $null; $raw = $null; $tokens = $null; $keep = $null; $line = $null; $cur = $null; $before = $null
  for ($i = 0; $i -lt 100; $i++) {
    if ((Get-Content "$Dir\bridge.out.log" -ErrorAction SilentlyContinue) -match 'waiting to speak') { return $bridge }   # the game has dialled (it speaks only once a map loads)
    Start-Sleep 3
  }
  throw 'the game did not connect to the bridge'
}

function Start-RlBot([string]$Dir, [string]$AgentId, [string]$Tag, $Owned) {
  $env:RLBOT_SERVER_PORT = '23234'; $env:RLBOT_AGENT_ID = $AgentId
  $process = Start-Process "$script:Bin\rb_chaser_bot.exe" -PassThru -WindowStyle Hidden `
    -RedirectStandardOutput "$Dir\bot-$Tag.out.log" -RedirectStandardError "$Dir\bot-$Tag.err.log"
  Add-RlOwnedProcess $Owned $process
}

function Get-RlWindow {
  $p = $script:OwnedGame
  if (-not $p -or $p.HasExited -or $p.MainWindowHandle -eq 0) { throw 'no Rocket League window' }
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
