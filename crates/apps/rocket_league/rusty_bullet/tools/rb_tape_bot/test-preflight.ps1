# Execute the real preflight.ps1 in isolated layouts with a stubbed cmake. No game, BakkesMod or desktop.
$ErrorActionPreference = 'Stop'
function Assert($Condition, [string]$Message) { if (-not $Condition) { throw $Message } }
$preflight = Join-Path $PSScriptRoot 'preflight.ps1'
$temp = Join-Path ([IO.Path]::GetTempPath()) ('rb preflight tests ' + [guid]::NewGuid())
$originalLocation = Get-Location

function New-Layout([string]$name) {
    $layout = Join-Path $temp $name
    $bm = Join-Path $layout 'bakkesmod'
    $src = Join-Path $layout 'plugin source'
    $null = New-Item -ItemType Directory -Force (Join-Path $bm 'plugins'), (Join-Path $bm 'bakkesmodsdk\lib'), $src
    Set-Content (Join-Path $bm 'version.txt') '229'
    Set-Content (Join-Path $bm 'bakkesmodsdk\lib\pluginsdk.lib') 'sdk lib v1'
    foreach ($f in 'CMakeLists.txt', 'RustyBulletCapturePlugin.cpp', 'RustyBulletCapturePlugin.h') {
        Set-Content (Join-Path $src $f) "$f v1"
    }
    [pscustomobject]@{ Bm = $bm; Src = $src; Plugins = Join-Path $bm 'plugins'; Layout = $layout }
}

# Run preflight; returns what it did. Any throw is captured, not propagated.
function Invoke-Preflight($l, [switch]$Check, [int]$CmakeExit = 0, [switch]$GameRunning, [switch]$CloseGame, [switch]$GameStaysUp) {
    $global:cmakeCalls = [Collections.Generic.List[string]]::new()
    $global:cmakeExit = $CmakeExit
    $global:configureArgs = @()
    $global:gameRunning = [bool]$GameRunning
    $global:gameStaysUp = [bool]$GameStaysUp
    $global:stopCalls = 0
    $failure = $null
    $args2 = @{ BakkesModDir = $l.Bm; PluginSource = $l.Src; GameDir = Join-Path $l.Layout 'no game'; RlbotDir = Join-Path $l.Layout 'no rlbot' }
    if ($Check) { $args2.Check = $true }
    if ($CloseGame) { $args2.CloseGame = $true; $args2.CloseTimeoutSec = 1 }
    try { & $preflight @args2 6>$null } catch { $failure = $_ }
    [pscustomobject]@{ Failure = $failure; Calls = ($global:cmakeCalls -join ','); Stops = $global:stopCalls; Configure = $global:configureArgs }
}

function cmake {
    $global:cmakeCalls.Add($(if ($args -contains '--build') { 'build' } else { 'configure' }))
    if ($args -notcontains '--build') { $global:configureArgs = @($args) }
    $global:LASTEXITCODE = $global:cmakeExit
    if ($global:cmakeExit -eq 0 -and $args -contains '--build') {
        $out = Join-Path (Get-Location) 'build\Release'
        $null = New-Item -ItemType Directory -Force $out
        Set-Content (Join-Path $out 'rusty_bullet_capture.dll') ("dll built from " + (Get-Content (Join-Path (Get-Location) 'RustyBulletCapturePlugin.cpp')))
    }
}
function Stop-Process {
    $global:stopCalls++
    if (-not $global:gameStaysUp) { $global:gameRunning = $false }
}
function Get-Process {
    if ($global:gameRunning) { [pscustomobject]@{ Name = 'RocketLeague' } }
}

try {
    # fresh install: no DLL, no stamp -> builds, installs, stamps
    $l = New-Layout 'fresh'
    $r = Invoke-Preflight $l
    Assert ($null -eq $r.Failure) "fresh failed: $($r.Failure)"
    Assert ($r.Calls -eq 'configure,build') "fresh wrong cmake calls: $($r.Calls)"
    Assert (Test-Path (Join-Path $l.Plugins 'rusty_bullet_capture.dll')) 'fresh did not install the DLL'
    $stamp = Get-Content (Join-Path $l.Plugins 'rusty_bullet_capture.stamp.json') -Raw | ConvertFrom-Json
    Assert ($stamp.bakkesmod_version -eq '229') 'stamp lost the BakkesMod version'
    Write-Host 'PASS fresh'

    # The SDK that is fingerprinted is the SDK CMake links: the selected -BakkesModDir, not the
    # %APPDATA% default in CMakeLists.txt. This layout is off the default and its path has spaces.
    $expectedSdk = (Join-Path $l.Bm 'bakkesmodsdk') -replace '\\', '/'
    Assert ($expectedSdk -match ' ') 'test layout should have spaces in the SDK path'
    Assert ($expectedSdk -ne ((Join-Path $env:APPDATA 'bakkesmod\bakkesmod\bakkesmodsdk') -replace '\\', '/')) 'test layout should not be the APPDATA default'
    Assert (@($r.Configure) -ccontains "-DBAKKESMOD_SDK_PATH=$expectedSdk") "configure did not pass the selected SDK: $($r.Configure -join ' | ')"
    Write-Host 'PASS sdk-path-passed'

    # same inputs again -> nothing to do
    $r = Invoke-Preflight $l
    Assert ($null -eq $r.Failure) "current failed: $($r.Failure)"
    Assert ($r.Calls -eq '') "current rebuilt anyway: $($r.Calls)"
    Write-Host 'PASS current'

    # -Check on a current plugin passes and changes nothing
    $r = Invoke-Preflight $l -Check
    Assert ($null -eq $r.Failure -and $r.Calls -eq '') "check on current: $($r.Failure)"
    Write-Host 'PASS check-current'

    # BakkesMod updated -> rebuild, old DLL backed up under its own version, restamped
    Set-Content (Join-Path $l.Bm 'version.txt') '230'
    $before = Get-Content (Join-Path $l.Plugins 'rusty_bullet_capture.dll') -Raw
    $r = Invoke-Preflight $l -Check
    Assert ($null -ne $r.Failure -and $r.Failure -match 'stale' -and $r.Failure -match '229' -and $r.Calls -eq '') "check on stale: $($r.Failure) / $($r.Calls)"
    $r = Invoke-Preflight $l
    Assert ($null -eq $r.Failure -and $r.Calls -eq 'configure,build') "bm update: $($r.Failure) / $($r.Calls)"
    Assert ((Get-Content (Join-Path $l.Plugins 'rusty_bullet_capture.bm229.dll.bak') -Raw) -eq $before) 'old DLL not backed up as bm229'
    Assert ((Get-Content (Join-Path $l.Plugins 'rusty_bullet_capture.stamp.json') -Raw | ConvertFrom-Json).bakkesmod_version -eq '230') 'not restamped'
    Write-Host 'PASS bakkesmod-updated'

    # SDK lib changed under the same version string -> rebuild
    Set-Content (Join-Path $l.Bm 'bakkesmodsdk\lib\pluginsdk.lib') 'sdk lib v2'
    $r = Invoke-Preflight $l
    Assert ($null -eq $r.Failure -and $r.Calls -eq 'configure,build') "sdk lib change: $($r.Failure) / $($r.Calls)"
    Write-Host 'PASS sdk-lib-changed'

    # plugin source changed -> rebuild
    Set-Content (Join-Path $l.Src 'RustyBulletCapturePlugin.cpp') 'cpp v2'
    $r = Invoke-Preflight $l
    Assert ($null -eq $r.Failure -and $r.Calls -eq 'configure,build') "source change: $($r.Failure) / $($r.Calls)"
    Assert ((Get-Content (Join-Path $l.Plugins 'rusty_bullet_capture.dll') -Raw) -match 'cpp v2') 'new source not installed'
    Write-Host 'PASS source-changed'

    # a hand-built DLL with no stamp is not trusted
    $l = New-Layout 'unstamped'
    Set-Content (Join-Path $l.Plugins 'rusty_bullet_capture.dll') 'hand built'
    $r = Invoke-Preflight $l -Check
    Assert ($null -ne $r.Failure -and $r.Failure -match 'no build stamp') "unstamped check: $($r.Failure)"
    $r = Invoke-Preflight $l
    Assert ($null -eq $r.Failure) "unstamped rebuild: $($r.Failure)"
    Assert ((Get-Content (Join-Path $l.Plugins 'rusty_bullet_capture.unstamped.dll.bak') -Raw).Trim() -eq 'hand built') 'unstamped DLL not backed up'
    Write-Host 'PASS unstamped'

    # failed build leaves the installed DLL and stamp alone
    $l = New-Layout 'build-failed'
    $r = Invoke-Preflight $l
    Set-Content (Join-Path $l.Bm 'version.txt') '230'
    $dll = Join-Path $l.Plugins 'rusty_bullet_capture.dll'
    $stampPath = Join-Path $l.Plugins 'rusty_bullet_capture.stamp.json'
    $dllBefore = Get-Content $dll -Raw; $stampBefore = Get-Content $stampPath -Raw
    $r = Invoke-Preflight $l -CmakeExit 7
    Assert ($null -ne $r.Failure -and $r.Failure -match 'cmake configure failed.*7') "build-failed message: $($r.Failure)"
    Assert ((Get-Content $dll -Raw) -eq $dllBefore -and (Get-Content $stampPath -Raw) -eq $stampBefore) 'failed build touched the install'
    Write-Host 'PASS build-failed'

    # game running -> refuse to replace the DLL it holds
    $r = Invoke-Preflight $l -GameRunning
    Assert ($null -ne $r.Failure -and $r.Failure -match 'running' -and $r.Calls -eq '') "game running: $($r.Failure) / $($r.Calls)"
    Write-Host 'PASS game-running'

    # game running and -Check/-CloseGame absent: refuse and never stop the game
    Assert ($r.Stops -eq 0) 'refusal without -CloseGame stopped the game'

    # -CloseGame stops the game, but only because a rebuild is needed
    $r = Invoke-Preflight $l -GameRunning -CloseGame
    Assert ($null -eq $r.Failure -and $r.Calls -eq 'configure,build' -and $r.Stops -eq 1) "closegame: $($r.Failure) / $($r.Calls) / stops $($r.Stops)"
    Write-Host 'PASS closegame-when-stale'

    # current plugin: the game is left running even with -CloseGame
    $r = Invoke-Preflight $l -GameRunning -CloseGame
    Assert ($null -eq $r.Failure -and $r.Calls -eq '' -and $r.Stops -eq 0) "closegame on current plugin: $($r.Failure) / stops $($r.Stops)"
    Write-Host 'PASS closegame-skipped-when-current'

    # -Check never changes anything, including the running game
    Set-Content (Join-Path $l.Bm 'version.txt') '231'
    $r = Invoke-Preflight $l -Check -GameRunning -CloseGame
    Assert ($null -ne $r.Failure -and $r.Failure -match 'stale' -and $r.Stops -eq 0 -and $r.Calls -eq '') "closegame with -Check: $($r.Failure) / stops $($r.Stops)"
    Write-Host 'PASS closegame-ignored-by-check'

    # a failed build must not end the session: the game is only stopped once there is a DLL to install
    $dllBefore = Get-Content $dll -Raw
    $r = Invoke-Preflight $l -GameRunning -CloseGame -CmakeExit 7
    Assert ($null -ne $r.Failure -and $r.Failure -match 'cmake configure failed.*7' -and $r.Stops -eq 0) "failed build with -CloseGame: $($r.Failure) / stops $($r.Stops)"
    Assert ((Get-Content $dll -Raw) -eq $dllBefore) 'failed build with -CloseGame changed the install'
    Write-Host 'PASS closegame-build-failed-keeps-game'

    # game that will not exit: built, then fail without touching the install
    $r = Invoke-Preflight $l -GameRunning -CloseGame -GameStaysUp
    Assert ($null -ne $r.Failure -and $r.Failure -match 'did not exit' -and $r.Calls -eq 'configure,build' -and $r.Stops -eq 1) "game stays up: $($r.Failure) / $($r.Calls)"
    Assert ((Get-Content $dll -Raw) -eq $dllBefore) 'game that would not exit still changed the install'
    Write-Host 'PASS closegame-game-wont-exit'

    # unreadable stamp is an error, not "absent"
    Set-Content $stampPath '{broken'
    $r = Invoke-Preflight $l
    Assert ($null -ne $r.Failure -and $r.Failure -match 'unreadable' -and $r.Calls -eq '') "bad stamp: $($r.Failure)"
    Write-Host 'PASS unreadable-stamp'

    # BakkesMod not where expected
    $l = New-Layout 'no-bakkesmod'
    Remove-Item (Join-Path $l.Bm 'version.txt')
    $r = Invoke-Preflight $l
    Assert ($null -ne $r.Failure -and $r.Failure -match 'version.txt' -and $r.Calls -eq '') "no bakkesmod: $($r.Failure)"
    Write-Host 'PASS no-bakkesmod'

    Assert ((Get-Location).Path -eq $originalLocation.Path) 'leaked current directory'
} finally {
    Set-Location $originalLocation
    Remove-Item Function:cmake, Function:Get-Process, Function:Stop-Process -ErrorAction SilentlyContinue
    Remove-Variable -Scope Global cmakeCalls, cmakeExit, gameRunning, gameStaysUp, stopCalls -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $temp -Recurse -Force -ErrorAction SilentlyContinue
}
