# Execute the real ScoreOnly entry point in isolated layouts. No game or desktop.
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
function Assert($Condition, [string]$Message) { if (-not $Condition) { throw $Message } }
$pwsh = (Get-Process -Id $PID).Path
$temp = Join-Path ([IO.Path]::GetTempPath()) ('rb batch tests ' + [guid]::NewGuid())
$oldTarget = $env:CARGO_TARGET_DIR
$oldVerify = $env:RB_VERIFY
$originalLocation = Get-Location
try {
    foreach ($case in @('workspace', 'override', 'standalone', 'build-failed', 'metadata-failed', 'malformed', 'empty', 'absent', 'wrong-type', 'missing', 'directory')) {
        $layout = Join-Path $temp $case
        $bot = Join-Path $layout 'tools/rb_tape_bot'
        $batch = Join-Path $layout 'capture batch'
        $null = New-Item -ItemType Directory -Force $bot, $batch
        Copy-Item (Join-Path $PSScriptRoot 'run_batch.ps1') $bot
        $target = Join-Path $layout 'workspace target'
        $env:CARGO_TARGET_DIR = $null
        if ($case -eq 'override') {
            $env:CARGO_TARGET_DIR = Join-Path $layout 'overridden target with spaces'
            $target = $env:CARGO_TARGET_DIR
        }
        $binary = Join-Path $target 'release/rb-verify.exe'
        if ($case -eq 'standalone') { $binary = Join-Path $layout 'target/release/rb-verify.exe' }
        $null = New-Item -ItemType Directory -Force (Split-Path $binary)
        if ($case -eq 'directory') { $null = New-Item -ItemType Directory $binary }
        elseif ($case -ne 'missing') { Set-Content $binary 'stale executable fixture - must never run' }
        $calls = [Collections.Generic.List[string]]::new()
        # Native exit status is produced by a real child, not ErrorActionPreference.
        function cargo {
            $calls.Add($args[0])
            if ($args[0] -eq 'build') {
                & $pwsh -NoProfile -Command $(if ($case -eq 'build-failed') { 'exit 23' } else { 'exit 0' })
            } else {
                & $pwsh -NoProfile -Command $(if ($case -eq 'metadata-failed') { 'exit 24' } else { 'exit 0' })
                switch ($case) {
                    'malformed' { '{broken' }
                    'empty' { '{"target_directory":""}' }
                    'absent' { '{}' }
                    'wrong-type' { '{"target_directory":42}' }
                    default { @{ target_directory = $target } | ConvertTo-Json -Compress }
                }
            }
            $global:LASTEXITCODE = $LASTEXITCODE
        }
        $failure = $null
        try { & (Join-Path $bot 'run_batch.ps1') -ScoreOnly $batch } catch { $failure = $_ }
        Assert ((Get-Location).Path -eq $originalLocation.Path) "$case leaked current directory"
        if ($case -in @('workspace', 'override', 'standalone')) {
            Assert ($null -eq $failure) "$case failed: $failure"
            Assert ($env:RB_VERIFY -eq $binary) "$case selected wrong binary"
            Assert (Test-Path (Join-Path $batch 'results.md')) "$case did not finish scoring"
            $expected = if ($case -eq 'standalone') { '' } else { 'build,metadata' }
            Assert (($calls -join ',') -eq $expected) "$case wrong cargo calls: $($calls -join ',')"
        } else {
            Assert ($null -ne $failure) "$case must fail before scoring"
            Assert (-not (Test-Path (Join-Path $batch 'results.md'))) "$case scored after failure"
            if ($case -eq 'build-failed') {
                Assert (($calls -join ',') -eq 'build') 'metadata masked the failed build'
                Assert ($failure -match 'cargo build.*23') 'build exit code lost'
            }
            if ($case -eq 'metadata-failed') { Assert ($failure -match 'cargo metadata.*24') 'metadata exit code lost' }
        }
        Write-Host "PASS $case"
    }
} finally {
    $env:CARGO_TARGET_DIR = $oldTarget
    $env:RB_VERIFY = $oldVerify
    Remove-Item Function:cargo -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $temp -Recurse -Force
}
