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

    # Only <name>_run<k>.jsonl are captures: rb_run_tapes also writes <name>_run<k>.pads.jsonl beside each one
    # (ADR-0071), and the scorer must not hand it to rb-verify. The report is identical either way under pwsh,
    # so this counts what rb-verify is asked to score, with a compiled stand-in that logs its --against argument.
    $layout = Join-Path $temp 'sidecar'
    $bot = Join-Path $layout 'tools/rb_tape_bot'
    $batch = Join-Path $layout 'capture batch'
    $binary = Join-Path $layout 'target/release/rb-verify.exe'
    $log = Join-Path $layout 'verify-calls.log'
    $null = New-Item -ItemType Directory -Force $bot, $batch, (Split-Path $binary)
    Copy-Item (Join-Path $PSScriptRoot 'run_batch.ps1') $bot
    foreach ($name in 'scn_run1.jsonl', 'scn_run2.jsonl', 'scn_run1.pads.jsonl', 'scn_run2.pads.jsonl') {
        Set-Content (Join-Path $batch $name) ''
    }
    $stub = Join-Path $layout 'rb_verify_stub.rs'
    Set-Content $stub @'
use std::io::Write;
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let log = std::env::var("RB_STUB_LOG").expect("RB_STUB_LOG");
    let against = args.iter().position(|a| a == "--against").and_then(|i| args.get(i + 1)).expect("--against");
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(log).expect("open log");
    writeln!(f, "{against}").expect("write log");
}
'@
    rustc --edition 2021 -o $binary $stub
    Assert ($LASTEXITCODE -eq 0) 'could not compile the rb-verify stand-in'
    $env:RB_STUB_LOG = $log
    $env:CARGO_TARGET_DIR = $null
    & (Join-Path $bot 'run_batch.ps1') -ScoreOnly $batch
    $scored = @(Get-Content $log | ForEach-Object { Split-Path $_ -Leaf } | Sort-Object)
    Assert (($scored -join ',') -eq 'scn_run1.jsonl,scn_run2.jsonl') "scorer asked rb-verify to score: $($scored -join ', ')"
    Write-Host 'PASS sidecar'
} finally {
    $env:CARGO_TARGET_DIR = $oldTarget
    $env:RB_VERIFY = $oldVerify
    $env:RB_STUB_LOG = $null
    Remove-Item Function:cargo -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $temp -Recurse -Force
}
