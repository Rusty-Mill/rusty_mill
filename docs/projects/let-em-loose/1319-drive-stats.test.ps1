#Requires -Version 7.0
<#
  Tests for 1319-drive-stats.ps1 on disposable synthetic fixtures in a temp directory only.
  No real drive is read. No Pester needed:  pwsh -File .\1319-drive-stats.test.ps1
  Exits 1 on the first failure.
#>
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '1319-drive-stats.ps1')

# A stream that returns at most one byte per read: legal, and the case that made refunds unsafe.
class TrickleStream : System.IO.MemoryStream {
    TrickleStream([byte[]]$bytes) : base($bytes) {}
    [int] Read([byte[]]$buffer, [int]$offset, [int]$count) {
        return ([System.IO.MemoryStream]$this).Read($buffer, $offset, [math]::Min(1, $count))
    }
}

# A stream whose second read throws, after the first has consumed bytes.
class FailingStream : System.IO.MemoryStream {
    [int]$Calls = 0
    FailingStream([byte[]]$bytes) : base($bytes) {}
    [int] Read([byte[]]$buffer, [int]$offset, [int]$count) {
        $this.Calls++
        if ($this.Calls -gt 1) { throw [System.IO.IOException]::new('synthetic read failure') }
        return ([System.IO.MemoryStream]$this).Read($buffer, $offset, $count)
    }
}

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
    $groups = New-Groups @{ 100 = 2; 1000 = 3; 10000 = 4; 7 = 1 }   # costs (size+1) x n: 202, 3003, 40004; 7 is a singleton
    $rng = [System.Random]::new(1)
    $plan = Select-HashBudget -SizeGroups $groups -MaxBytes 3500 -MaxGroups 10 -Rng $rng
    Assert-Equal $plan.GroupsEligible 3 'singletons are not eligible'
    Assert-Equal ($plan.ProjectedBytes -le 3500) $true 'projected bytes stay within the budget'
    Assert-Equal $plan.Sizes.Contains(10000) $false 'a group costing more than the budget is never chosen'
    Assert-Equal $plan.ProjectedBytes 3205 'smaller groups still fill the budget around a skipped one'

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
    Assert-Equal $m.bytes_requested 4004 'bytes_requested counts every request, EOF probes included'

    # --- Guard: membership grew between passes ------------------------------------------------
    $opened = [ref]0
    $countingOpen = { param([string]$p) $opened.Value++; [System.IO.File]::OpenRead($p) }.GetNewClosure()
    $g = Measure-HashGroups -Members $members -MaxBytes 3000 -Open $countingOpen
    Assert-Equal $g.groups_skipped_budget 1 'a group that grew past the budget is skipped whole'
    Assert-Equal $opened.Value 0 'nothing is opened in a skipped group'
    Assert-Equal $g.bytes_requested 0 'bytes_requested stays zero'

    # --- Read-BoundedHash: the cap holds inside the read loop --------------------------------
    $data = [byte[]]::new(3000)
    $r = Read-BoundedHash -Stream ([System.IO.MemoryStream]::new($data)) -Expected 3000 -Budget @{ remaining = 3001; requested = 0L; returned = 0L } -ChunkBytes 1024
    Assert-Equal $r.Status 'ok' 'exact length within budget hashes'

    $bud = @{ remaining = 3010; requested = 0L; returned = 0L }
    $grown = [byte[]]::new(5000)                     # the file grew after the size check
    $r = Read-BoundedHash -Stream ([System.IO.MemoryStream]::new($grown)) -Expected 3000 -Budget $bud -ChunkBytes 1024
    Assert-Equal $r.Status 'changed' 'a file that grew during the read is changed, not a match'
    Assert-Equal $bud.requested 3001 'reading stops at expected + 1 byte, however much the file grew'

    $bud = @{ remaining = 2000; requested = 0L; returned = 0L }
    $r = Read-BoundedHash -Stream ([System.IO.MemoryStream]::new($grown)) -Expected 3000 -Budget $bud -ChunkBytes 1024
    Assert-Equal $r.Status 'budget' 'the read stops before it would pass the budget'
    Assert-Equal ($bud.requested -le 2000) $true 'bytes requested never exceed the budget'

    $bud = @{ remaining = 10000; requested = 0L; returned = 0L }
    $r = Read-BoundedHash -Stream ([System.IO.MemoryStream]::new([byte[]]::new(1500))) -Expected 3000 -Budget $bud -ChunkBytes 1024
    Assert-Equal $r.Status 'incomplete' 'a file that ends early is incomplete, not a match'

    $bud = @{ remaining = 10000; requested = 0L; returned = 0L }
    $r = Read-BoundedHash -Stream ([FailingStream]::new([byte[]]::new(3000))) -Expected 3000 -Budget $bud -ChunkBytes 1024
    Assert-Equal $r.Status 'failed' 'a read that throws is failed'
    Assert-Equal $bud.requested 2048 'a failed read stays charged: 1024 read plus the 1024 requested when it threw'
    Assert-Equal $bud.returned 1024 'only the bytes actually returned count as returned'

    $bud = @{ remaining = 5; requested = 0L; returned = 0L }
    $r = Read-BoundedHash -Stream ([TrickleStream]::new([byte[]]::new(4))) -Expected 4 -Budget $bud -ChunkBytes 4
    Assert-Equal ($bud.requested -le 5) $true 'short reads cannot reuse budget: requests stay within the cap'
    Assert-Equal $r.Status 'budget' 'a trickling stream runs out of budget instead of over-reading'

    # --- Overlapping roots are refused --------------------------------------------------------
    $sub = Join-Path $root 'sub'
    New-Item -ItemType Directory -Path $sub | Out-Null
    $threw = $false
    try { Assert-DisjointRoots @($root, $sub) } catch { $threw = $true }
    Assert-Equal $threw $true 'a root inside another root is refused'
    $threw = $false
    try { Assert-DisjointRoots @($root, (Join-Path $root '.')) } catch { $threw = $true }
    Assert-Equal $threw $true 'the same root given twice is refused'
    $sibling = Join-Path ([System.IO.Path]::GetTempPath()) ((Split-Path $root -Leaf) + '-x')
    $threw = $false
    try { Assert-DisjointRoots @($root, $sibling) } catch { $threw = $true }
    Assert-Equal $threw $false 'a sibling sharing a name prefix is not an overlap'

    # Windows and UNC forms, checked as strings so they run on any OS.
    $ci = [System.StringComparison]::OrdinalIgnoreCase
    Assert-Equal (Get-RootBoundary 'D:\') 'D:/' 'a drive root keeps exactly one boundary separator'
    Assert-Equal (Get-RootBoundary 'D:\Photos\') 'D:/Photos/' 'trailing separators collapse to one'
    Assert-Equal ($null -ne (Find-RootOverlap @('D:\', 'D:\Photos') $ci)) $true 'drive root D:\ contains D:\Photos'
    Assert-Equal ($null -ne (Find-RootOverlap @('d:\photos', 'D:\') $ci)) $true 'overlap is found in either order and any case'
    Assert-Equal ($null -ne (Find-RootOverlap @('D:\', 'D:') $ci)) $true 'D:\ and D: are the same root'
    Assert-Equal ($null -ne (Find-RootOverlap @('D:\', 'E:\') $ci)) $false 'different drives do not overlap'
    Assert-Equal ($null -ne (Find-RootOverlap @('\\nas\share', '\\nas\share\Photos') $ci)) $true 'a UNC share root contains its child'
    Assert-Equal ($null -ne (Find-RootOverlap @('\\nas\share\', '\\nas\share') $ci)) $true 'a UNC share root given twice'
    Assert-Equal ($null -ne (Find-RootOverlap @('\\nas\share', '\\nas\share2') $ci)) $false 'UNC shares sharing a name prefix do not overlap'
    Assert-Equal ($null -ne (Find-RootOverlap @('/', '/tmp') ([System.StringComparison]::Ordinal))) $true 'the Unix root contains everything'

    # --- Measure-HashGroups keeps grown, short and failed files out of the matches ----------
    $f1 = New-Fixture $root 'f1.bin' 2000 4
    $f2 = New-Fixture $root 'f2.bin' 2000 4
    $f3 = New-Fixture $root 'f3.bin' 2000 4
    $streams = @{ $f1 = { [System.IO.MemoryStream]::new([byte[]]::new(2500)) }; $f2 = { [FailingStream]::new([byte[]]::new(2000)) } }
    $fakeOpen = { param([string]$p) if ($streams.ContainsKey($p)) { & $streams[$p] } else { [System.IO.File]::OpenRead($p) } }.GetNewClosure()
    $mixed = Measure-HashGroups -Members @{ [long]2000 = [System.Collections.Generic.List[string]]@($f1, $f2, $f3) } -MaxBytes 1000000 -Open $fakeOpen
    Assert-Equal $mixed.files_changed 1 'grown during read: counted as changed'
    Assert-Equal $mixed.files_failed 1 'failed read: counted as failed'
    Assert-Equal $mixed.files_hashed 1 'only the clean read is hashed'
    Assert-Equal $mixed.path_duplicate_files 0 'grown and failed files are never matched'

    # --- Guard: a file changed size between passes -------------------------------------------
    $e1 = New-Fixture $root 'e1.bin' 500 3
    $e2 = New-Fixture $root 'e2.bin' 500 3
    $e3 = New-Fixture $root 'e3.bin' 500 3
    [System.IO.File]::WriteAllBytes($e3, [byte[]]::new(800))     # grew after "pass 2" found it at 500
    $changed = Measure-HashGroups -Members @{ [long]500 = [System.Collections.Generic.List[string]]@($e1, $e2, $e3) } -MaxBytes 1000000
    Assert-Equal $changed.files_changed 1 'a file whose size changed is not read'
    Assert-Equal $changed.bytes_requested 1002 'only the unchanged files are read (500 + 1 probe byte each)'
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
    Assert-Equal ($report.sample_hash.bytes_requested -le 1500) $true 'end to end: bytes read stay within the budget'
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
