#Requires -Version 7.0
<#
  Tests for 1319-drive-stats.ps1 on disposable synthetic fixtures in a temp directory only.
  No real drive is read. No Pester needed:  pwsh -File .\1319-drive-stats.test.ps1
  Exits 1 on the first failure.
#>
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '1319-drive-stats.ps1')

$script:failures = 0
function Assert-Equal($actual, $expected, [string]$what) {
    if ($actual -eq $expected) { Write-Host "ok   $what"; return }
    Write-Host "FAIL $what : expected $expected, got $actual"; $script:failures++
}

function New-Fixture([string]$dir, [string]$name, [int]$size, [byte]$fill) {
    $path = Join-Path $dir $name
    $buf = [byte[]]::new($size)
    [System.Array]::Fill($buf, $fill)
    [System.IO.File]::WriteAllBytes($path, $buf)
    return $path
}

function New-Groups([hashtable]$counts) {
    $d = [System.Collections.Generic.Dictionary[long, int]]::new()
    foreach ($k in $counts.Keys) { $d[[long]$k] = [int]$counts[$k] }
    return $d
}

$root = Join-Path ([System.IO.Path]::GetTempPath()) ("lel-1319-test-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $root | Out-Null
try {
    # --- Select-HashBudget: whole groups, never over the budget -------------------------------
    $groups = New-Groups @{ 100 = 2; 1000 = 3; 10000 = 4; 7 = 1 }   # costs 200, 3000, 40000; size 7 is a singleton
    $rng = [System.Random]::new(1)
    $plan = Select-HashBudget -SizeGroups $groups -MaxBytes 3500 -MaxGroups 10 -Rng $rng
    Assert-Equal $plan.GroupsEligible 3 'singletons are not eligible'
    Assert-Equal ($plan.ProjectedBytes -le 3500) $true 'projected bytes stay within the budget'
    Assert-Equal $plan.Sizes.Contains(10000) $false 'a group costing more than the budget is never chosen'
    Assert-Equal $plan.ProjectedBytes 3200 'smaller groups still fill the budget around a skipped one'

    $plan0 = Select-HashBudget -SizeGroups $groups -MaxBytes 0 -MaxGroups 10 -Rng ([System.Random]::new(1))
    Assert-Equal $plan0.Sizes.Count 0 'a zero budget selects nothing'

    $plan1 = Select-HashBudget -SizeGroups $groups -MaxBytes 1000000 -MaxGroups 1 -Rng ([System.Random]::new(1))
    Assert-Equal $plan1.Sizes.Count 1 'MaxGroups caps the group count'

    # --- Measure-HashGroups: path-based duplicates, hardlinks counted --------------------------
    $a = New-Fixture $root 'a.bin' 1000 1
    $b = New-Fixture $root 'b.bin' 1000 1          # byte-identical to a
    $c = New-Fixture $root 'c.bin' 1000 2          # same size, different bytes (decoy)
    $d = Join-Path $root 'd.bin'
    New-Item -ItemType HardLink -Path $d -Target $a | Out-Null   # same storage as a

    $members = @{ [long]1000 = [System.Collections.Generic.List[string]]@($a, $b, $c, $d) }
    $m = Measure-HashGroups -Members $members -MaxBytes 1000000
    Assert-Equal $m.files_hashed 4 'all four paths hashed'
    Assert-Equal $m.path_duplicate_files 3 'a, b and the hardlink d are path duplicates; c is not'
    Assert-Equal $m.path_duplicate_bytes 2000 'path_duplicate_bytes counts the hardlink, so it is not reclaimable space'
    Assert-Equal $m.bytes_read 4000 'bytes_read is what was hashed'

    # --- Guard: membership grew between passes ------------------------------------------------
    $calls = [ref]0
    $counting = { param([string]$p) $calls.Value++; (Get-FileHash -LiteralPath $p -Algorithm SHA256).Hash }.GetNewClosure()
    $g = Measure-HashGroups -Members $members -MaxBytes 3000 -Hasher $counting
    Assert-Equal $g.groups_skipped_budget 1 'a group that grew past the budget is skipped whole'
    Assert-Equal $calls.Value 0 'nothing is read from a skipped group'
    Assert-Equal $g.bytes_read 0 'bytes_read stays zero'

    # --- Guard: a file changed size between passes -------------------------------------------
    $e1 = New-Fixture $root 'e1.bin' 500 3
    $e2 = New-Fixture $root 'e2.bin' 500 3
    $e3 = New-Fixture $root 'e3.bin' 500 3
    [System.IO.File]::WriteAllBytes($e3, [byte[]]::new(800))     # grew after "pass 2" found it at 500
    $changed = Measure-HashGroups -Members @{ [long]500 = [System.Collections.Generic.List[string]]@($e1, $e2, $e3) } -MaxBytes 1000000
    Assert-Equal $changed.files_changed 1 'a file whose size changed is not read'
    Assert-Equal $changed.bytes_read 1000 'only the unchanged files are read'
    Assert-Equal $changed.path_duplicate_files 2 'the unchanged pair is still found'

    # --- End to end on the fixture tree ------------------------------------------------------
    $out = Join-Path ([System.IO.Path]::GetTempPath()) ("lel-1319-out-" + [guid]::NewGuid() + '.json')
    Invoke-DriveStats -Roots $root -Out $out -TopExtensions 25 -MinExtensionCount 1 -SampleGroups 10
    $json = Get-Content -LiteralPath $out -Raw
    $report = $json | ConvertFrom-Json
    Assert-Equal $report.sample_hash $null 'metadata-only by default: no hashing'
    Assert-Equal $json.Contains('a.bin') $false 'no file names in the output'
    Assert-Equal $json.Contains($root) $false 'no paths in the output'
    Assert-Equal $json.Contains((ConvertTo-Json $root).Trim('"')) $false 'no JSON-escaped paths in the output'

    Invoke-DriveStats -Roots $root -Out $out -TopExtensions 25 -MinExtensionCount 1 -SampleGroups 10 `
        -SampleHash -MaxHashBytes 1500 -Rng ([System.Random]::new(1))
    $report = Get-Content -LiteralPath $out -Raw | ConvertFrom-Json
    Assert-Equal ($report.sample_hash.bytes_read -le 1500) $true 'end to end: bytes read stay within the budget'
    Assert-Equal ($report.sample_hash.projected_bytes -le 1500) $true 'end to end: projected read reported within the budget'
    Remove-Item -LiteralPath $out

    $threw = $false
    try { Invoke-DriveStats -Roots $root -Out $out -TopExtensions 25 -MinExtensionCount 1 -SampleGroups 10 -SampleHash -MaxHashBytes 0 }
    catch { $threw = $true }
    Assert-Equal $threw $true '-SampleHash without a byte budget is refused'
} finally {
    Remove-Item -LiteralPath $root -Recurse -Force
}

if ($script:failures -gt 0) { Write-Host "$($script:failures) failure(s)"; exit 1 }
Write-Host 'all passed'
