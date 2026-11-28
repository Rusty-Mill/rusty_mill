#Requires -Version 7.2
<#
.SYNOPSIS
  Read-only aggregate shape of the given roots, for Let Em Loose card #1319.
.DESCRIPTION
  Walks each root without following reparse points (symlinks, junctions) and writes one JSON file of
  counts and histograms. It writes no file names, paths or hashes (D243). By default it reads metadata
  only.

  -SampleHash opts in to hashing a random sample of same-size groups, to estimate how many paths have a
  byte-identical sibling. It needs -MaxHashBytes, a read budget the caller picks: whole groups are
  chosen against the metadata from the first pass until the budget is used, the projected read is
  printed and reported, and the budget is enforced inside the read loop itself: every read request is
  charged in full before it is issued and never refunded, including short, probe and failed reads, and a file that grows, ends early or fails is counted
  apart and never matched. The cap counts bytes the script requests; files are opened unbuffered so
  .NET adds no read-ahead, but the operating system may still prefetch beyond a read. Hashes stay in memory; only counts are written. Nothing on the
  scanned roots is modified: the report must be a new file in an existing folder outside every root
  (an existing file or a path inside a root is refused, and the file is created exclusively, never
  overwritten). "Outside" is checked both as typed and with every symlink and junction along the
  report folder and the roots resolved, so a link pointing into a root is refused. Roots must not
  contain or equal one another, checked the same two ways, so no tree is counted twice. Not resolved:
  bind mounts, subst drives, a mapped share of a local folder, and 8.3 short names; give the report
  an ordinary folder path, not one of those. Device paths (\\?\ and \\.\ forms) are refused for
  roots and the report.

  The duplicate figures are path-based: two hardlinks to one file count as two duplicate paths though
  they share storage, so they are not reclaimable space.
.NOTES
  Needs PowerShell 7.2 or later (.NET 6): Convert.ToHexString and unbuffered FileStream (bufferSize 1)
  are not available on 7.0/7.1, and both are out of support.
.EXAMPLE
  pwsh -File .\1319-drive-stats.ps1 -Roots 'D:\','\\nas\share' -Out "$HOME\lel-drive-stats.json"
.EXAMPLE
  pwsh -File .\1319-drive-stats.ps1 -Roots 'D:\' -SampleHash -MaxHashBytes 20GB
#>
param(
    [string[]]$Roots,
    [string]$Out = (Join-Path $HOME 'lel-drive-stats.json'),
    [int]$TopExtensions = 25,
    [int]$MinExtensionCount = 10,
    [switch]$SampleHash,
    [long]$MaxHashBytes = 0,
    [int]$SampleGroups = 2000
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$PathDuplicateNote = 'Path-based: hardlinked names count as duplicates though they share storage; not reclaimable space.'

function Get-Log2Bucket([long]$n) {
    # Bucket b holds values in [2^(b-1), 2^b); bucket 0 holds 0.
    if ($n -le 0) { return 0 }
    return [int][math]::Floor([math]::Log($n, 2)) + 1
}

function Add-Count([hashtable]$h, $key, [long]$by = 1) {
    if ($h.ContainsKey($key)) { $h[$key] += $by } else { $h[$key] = $by }
}

function Get-RootKind([string]$root) {
    if ($root.StartsWith('\\')) { return 'network' }
    $drive = [System.IO.DriveInfo]::new([System.IO.Path]::GetPathRoot($root))
    if ($drive.DriveType -eq [System.IO.DriveType]::Network) { return 'network' }
    return 'local'
}

function Get-RootBoundary([string]$FullPath) {
    # One canonical form with exactly one trailing boundary: '\' becomes '/', trailing separators are
    # trimmed, then one '/' is added. 'D:\' -> 'D:/', 'D:\Photos\' -> 'D:/Photos/', '\\nas\share' ->
    # '//nas/share/', '/' -> '/'. Pure, so Windows forms can be tested on any OS.
    return $FullPath.Replace('\', '/').TrimEnd('/') + '/'
}

function Assert-PlainPath([string]$Path) {
    # Pure: device-namespace forms (\\?\D:\x, \\.\D:\x, \\?\UNC\...) name the same folders as plain
    # paths but compare differently, so the overlap and outside-roots checks could miss them. Refuse them.
    $p = $Path.Replace('\', '/')
    if ($p.StartsWith('//?/') -or $p.StartsWith('//./')) { throw "Device path $Path is not supported. Use the plain form, such as D:\Data or \\nas\share." }
}

function Find-RootOverlap([string[]]$FullPaths, [System.StringComparison]$Comparison) {
    # Pure: returns the first pair (outer, inner) where one root contains or equals another, else $null.
    $keys = @($FullPaths | ForEach-Object { Get-RootBoundary $_ })
    for ($i = 0; $i -lt $keys.Count; $i++) {
        for ($j = 0; $j -lt $keys.Count; $j++) {
            if ($i -ne $j -and $keys[$j].StartsWith($keys[$i], $Comparison)) { return @($FullPaths[$i], $FullPaths[$j]) }
        }
    }
    return $null
}

function Assert-DisjointRoots([string[]]$Roots) {
    # Overlapping roots (D:\ and D:\Photos) would visit files twice and invent duplicates; refuse them.
    $full = @($Roots | ForEach-Object { [System.IO.Path]::GetFullPath($_) })
    $cmp = if ($IsWindows) { [System.StringComparison]::OrdinalIgnoreCase } else { [System.StringComparison]::Ordinal }
    $pair = Find-RootOverlap $full $cmp
    if ($null -ne $pair) { throw "Roots overlap: $($pair[0]) contains $($pair[1]). Give disjoint roots." }
    # A root given through a symlink or junction is walked at its target, so compare targets too.
    $real = @($Roots | ForEach-Object { Get-RealDirectory $_ })
    $pair = Find-RootOverlap $real $cmp
    if ($null -ne $pair) { throw "Roots overlap once links are resolved: $($pair[0]) contains $($pair[1]). Give disjoint roots." }
}

function Get-RealDirectory([string]$Path, [int]$Depth = 0) {
    # Resolves every symlink and junction along an existing directory path, one component at a time,
    # so an alias of a root resolves to the root itself. Not resolved: bind mounts, subst drives,
    # mapped shares of a local folder and 8.3 short names.
    if ($Depth -gt 40) { throw "Too many links resolving $Path." }
    $full = [System.IO.Path]::GetFullPath($Path)
    $real = [System.IO.Path]::GetPathRoot($full)
    $parts = $full.Substring($real.Length).Split([char[]]@('\', '/'), [System.StringSplitOptions]::RemoveEmptyEntries)
    foreach ($part in $parts) {
        $real = Join-Path $real $part
        if (-not (Test-Path -LiteralPath $real)) { continue }
        $target = [System.IO.DirectoryInfo]::new($real).ResolveLinkTarget($true)
        if ($null -ne $target) { $real = Get-RealDirectory $target.FullName ($Depth + 1) }
    }
    return $real
}

function Test-UnderAny([string[]]$Keys, [string[]]$Bounds, [System.StringComparison]$Comparison) {
    # Pure: true when any boundary key starts with any boundary prefix.
    foreach ($k in $Keys) {
        if (@($Bounds | Where-Object { $k.StartsWith($_, $Comparison) }).Count -gt 0) { return $true }
    }
    return $false
}

function Assert-OutputOutsideRoots([string]$Out, [string[]]$Roots, [System.StringComparison]$Comparison) {
    # The scanned roots are never written: the report must land in a new file outside every root,
    # compared both as typed and with symlinks and junctions on either side resolved.
    $outFull = [System.IO.Path]::GetFullPath($Out)
    $parent = [System.IO.Path]::GetDirectoryName($outFull)
    if (-not [System.IO.Directory]::Exists($parent)) { throw "-Out folder $parent does not exist. Give an existing folder." }
    $outKeys = @(
        (Get-RootBoundary $outFull),
        (Get-RootBoundary (Join-Path (Get-RealDirectory $parent) ([System.IO.Path]::GetFileName($outFull))))
    )
    foreach ($root in $Roots) {
        $rootKeys = @((Get-RootBoundary ([System.IO.Path]::GetFullPath($root))), (Get-RootBoundary (Get-RealDirectory $root)))
        if (Test-UnderAny $outKeys $rootKeys $Comparison) { throw "-Out $Out is inside scanned root $root. Write the report outside every root." }
    }
    if (Test-Path -LiteralPath $Out) { throw "-Out $Out already exists. Give a new file name; nothing is overwritten." }
}

function Write-NewFile([string]$Path, [string]$Text) {
    # CreateNew fails if the file appeared since the check, so an existing file is never truncated.
    $stream = [System.IO.File]::Open($Path, [System.IO.FileMode]::CreateNew, [System.IO.FileAccess]::Write, [System.IO.FileShare]::None)
    try {
        $writer = [System.IO.StreamWriter]::new($stream, [System.Text.UTF8Encoding]::new($false))
        try { $writer.Write($Text) } finally { $writer.Dispose() }
    } finally { $stream.Dispose() }
}

function Select-HashBudget {
    <#
      Pure: picks whole same-size groups at random until the byte budget is used.
      $SizeGroups maps file size to the number of files of that size (first-pass metadata).
      A group costs (size + 1) x count (the extra byte per file is the growth probe in Read-BoundedHash);
      groups over the remaining budget are skipped, smaller ones still fit.
    #>
    param(
        [System.Collections.Generic.Dictionary[long, int]]$SizeGroups,
        [long]$MaxBytes,
        [int]$MaxGroups,
        [System.Random]$Rng
    )
    $eligible = [System.Collections.Generic.List[long]]::new()
    foreach ($kv in $SizeGroups.GetEnumerator()) { if ($kv.Value -ge 2 -and $kv.Key -gt 0) { $eligible.Add($kv.Key) } }
    $eligible.Sort()
    for ($i = $eligible.Count - 1; $i -gt 0; $i--) {
        $j = $Rng.Next($i + 1)
        $tmp = $eligible[$i]; $eligible[$i] = $eligible[$j]; $eligible[$j] = $tmp
    }
    $chosen = [System.Collections.Generic.HashSet[long]]::new()
    $projected = 0L
    foreach ($size in $eligible) {
        if ($chosen.Count -ge $MaxGroups) { break }
        $cost = ($size + 1) * $SizeGroups[$size]
        if ($projected + $cost -gt $MaxBytes) { continue }
        [void]$chosen.Add($size); $projected += $cost
    }
    return [pscustomobject]@{ Sizes = $chosen; ProjectedBytes = $projected; GroupsEligible = $eligible.Count }
}

function Find-GroupMembers([string[]]$Roots, [System.Collections.Generic.HashSet[long]]$Sizes) {
    # Second pass: paths of files whose size is one of the chosen sizes. Paths stay in memory.
    $reparse = [System.IO.FileAttributes]::ReparsePoint
    $members = @{}
    foreach ($root in $Roots) {
        $stack = [System.Collections.Generic.Stack[System.IO.DirectoryInfo]]::new()
        $stack.Push([System.IO.DirectoryInfo]::new($root))
        while ($stack.Count -gt 0) {
            $dir = $stack.Pop()
            try {
                foreach ($e in $dir.EnumerateFileSystemInfos()) {
                    if ($e.Attributes -band $reparse) { continue }
                    if ($e -is [System.IO.DirectoryInfo]) { $stack.Push($e); continue }
                    if (-not $Sizes.Contains($e.Length)) { continue }
                    if (-not $members.ContainsKey($e.Length)) { $members[$e.Length] = [System.Collections.Generic.List[string]]::new() }
                    $members[$e.Length].Add($e.FullName)
                }
            } catch [System.UnauthorizedAccessException], [System.IO.IOException] { }
        }
    }
    return $members
}

function Read-BoundedHash {
    <#
      Hashes one stream, reading at most $Expected + 1 bytes and never past the shared budget.
      Every read request is charged in full before it is issued and never refunded: a short read, an EOF
      probe and a read that throws all stay charged, so the budget bounds the bytes the loop requests.
      Bytes actually returned are counted apart. After $Expected bytes it requests 1 more byte to detect
      growth.
      Status: ok (exactly $Expected bytes; Hash set), changed (more bytes), incomplete (fewer bytes),
      failed (a read threw), budget (the budget would be exceeded; nothing more is read).
    #>
    param([System.IO.Stream]$Stream, [long]$Expected, [hashtable]$Budget, [int]$ChunkBytes = 1MB)
    $sha = [System.Security.Cryptography.IncrementalHash]::CreateHash([System.Security.Cryptography.HashAlgorithmName]::SHA256)
    $buf = [byte[]]::new($ChunkBytes)
    $got = 0L
    try {
        while ($true) {
            $probe = $got -ge $Expected
            $want = if ($probe) { 1 } else { [int][math]::Min($ChunkBytes, $Expected - $got) }
            if ($Budget.remaining -lt $want) { return @{ Status = 'budget' } }
            $Budget.remaining -= $want; $Budget.requested += $want
            $n = $Stream.Read($buf, 0, $want)
            $Budget.returned += $n
            if ($probe) {
                if ($n -gt 0) { return @{ Status = 'changed' } }
                return @{ Status = 'ok'; Hash = [System.Convert]::ToHexString($sha.GetHashAndReset()) }
            }
            if ($n -eq 0) { return @{ Status = 'incomplete' } }
            $sha.AppendData($buf, 0, $n); $got += $n
        }
    } catch {
        return @{ Status = 'failed' }
    } finally {
        $sha.Dispose()
    }
}

function Measure-HashGroups {
    <#
      Hashes each group whole, never past the byte budget. A group whose current cost ((size + 1) x
      members found now) no longer fits is skipped. A file whose size changed before opening is not read;
      one that grows, ends early or fails while being read is counted apart and never matched.
      Returns counts only.
    #>
    param(
        [hashtable]$Members,
        [long]$MaxBytes,
        # bufferSize 1 turns off FileStream's own read-ahead buffer, so each read the loop issues is the
        # read that reaches the OS. Operating-system readahead below that is outside the script's control.
        [scriptblock]$Open = { param([string]$p) [System.IO.FileStream]::new($p, 'Open', 'Read', 'ReadWrite', 1) }
    )
    $budget = @{ remaining = $MaxBytes; requested = 0L; returned = 0L }
    $stats = [ordered]@{ groups_hashed = 0; groups_skipped_budget = 0; budget_exhausted = $false;
        files_hashed = 0L; files_changed = 0L; files_incomplete = 0L; files_failed = 0L;
        bytes_requested = 0L; bytes_returned = 0L; path_duplicate_files = 0L; path_duplicate_bytes = 0L }
    foreach ($size in ($Members.Keys | Sort-Object)) {
        $paths = $Members[$size]
        if ($paths.Count -lt 2) { continue }
        if (($size + 1) * $paths.Count -gt $budget.remaining) { $stats.groups_skipped_budget++; continue }
        $byHash = @{}
        foreach ($path in $paths) {
            $now = [System.IO.FileInfo]::new($path)
            if (-not $now.Exists -or $now.Length -ne $size) { $stats.files_changed++; continue }
            try { $stream = & $Open $path } catch { $stats.files_failed++; continue }
            try { $r = Read-BoundedHash -Stream $stream -Expected $size -Budget $budget } finally { $stream.Dispose() }
            switch ($r.Status) {
                'ok' { $stats.files_hashed++; Add-Count $byHash $r.Hash }
                'changed' { $stats.files_changed++ }
                'incomplete' { $stats.files_incomplete++ }
                'failed' { $stats.files_failed++ }
                'budget' { $stats.budget_exhausted = $true }
            }
            if ($stats.budget_exhausted) { break }
        }
        $stats.groups_hashed++
        foreach ($count in $byHash.Values) {
            if ($count -lt 2) { continue }
            $stats.path_duplicate_files += $count; $stats.path_duplicate_bytes += $size * ($count - 1)
        }
        if ($stats.budget_exhausted) { break }
    }
    $stats.bytes_requested = $budget.requested; $stats.bytes_returned = $budget.returned
    return $stats
}

function ConvertTo-SortedMap([hashtable]$h) {
    $m = [ordered]@{}
    foreach ($k in ($h.Keys | Sort-Object { [long]$_ })) { $m["$k"] = $h[$k] }
    return $m
}

function Invoke-DriveStats {
    param([string[]]$Roots, [string]$Out, [int]$TopExtensions, [int]$MinExtensionCount,
        [switch]$SampleHash, [long]$MaxHashBytes, [int]$SampleGroups, [System.Random]$Rng = [System.Random]::new())
    if (-not $Roots) { throw 'Give -Roots, e.g. -Roots D:\,\\nas\share' }
    if ($SampleHash -and $MaxHashBytes -le 0) { throw '-SampleHash needs -MaxHashBytes, a read budget such as 20GB' }
    foreach ($p in @($Roots) + $Out) { Assert-PlainPath $p }
    $missing = @($Roots | Where-Object { -not [System.IO.Directory]::Exists($_) })
    if ($missing.Count -gt 0) { throw "Root not found: $($missing[0])" }
    Assert-DisjointRoots $Roots
    $cmp = if ($IsWindows) { [System.StringComparison]::OrdinalIgnoreCase } else { [System.StringComparison]::Ordinal }
    Assert-OutputOutsideRoots $Out $Roots $cmp

    $reparse = [System.IO.FileAttributes]::ReparsePoint
    $sizeFiles = @{}; $sizeBytes = @{}; $depthHist = @{}; $fanHist = @{}; $extCount = @{}
    $sizeGroups = [System.Collections.Generic.Dictionary[long, int]]::new()
    $totals = [ordered]@{ files = 0L; bytes = 0L; dirs = 0L; zero_byte = 0L; reparse_points = 0L;
        unreadable_dirs = 0L; long_paths = 0L; max_depth = 0 }
    $rootRows = @()

    for ($i = 0; $i -lt $Roots.Count; $i++) {
        $root = $Roots[$i]
        $row = [ordered]@{ label = "root$($i + 1)"; kind = (Get-RootKind $root); files = 0L; bytes = 0L; dirs = 0L }
        $stack = [System.Collections.Generic.Stack[object]]::new()
        $stack.Push(@([System.IO.DirectoryInfo]::new($root), 0))
        while ($stack.Count -gt 0) {
            $dir, $depth = $stack.Pop()
            $entries = 0
            try {
                foreach ($e in $dir.EnumerateFileSystemInfos()) {
                    $entries++
                    if ($e.Attributes -band $reparse) { $totals.reparse_points++; continue }
                    if ($e.FullName.Length -gt 259) { $totals.long_paths++ }
                    if ($e -is [System.IO.DirectoryInfo]) {
                        $row.dirs++
                        $stack.Push(@($e, ($depth + 1)))
                        continue
                    }
                    $len = $e.Length
                    $row.files++; $row.bytes += $len
                    if ($len -eq 0) { $totals.zero_byte++ }
                    $b = Get-Log2Bucket $len
                    Add-Count $sizeFiles $b; Add-Count $sizeBytes $b $len
                    Add-Count $depthHist ($depth + 1)
                    if ($depth + 1 -gt $totals.max_depth) { $totals.max_depth = $depth + 1 }
                    $ext = $e.Extension.ToLowerInvariant()
                    if ($ext.Length -gt 11) { $ext = '(long)' }
                    Add-Count $extCount $ext
                    if ($len -gt 0) {
                        $n = 0; [void]$sizeGroups.TryGetValue($len, [ref]$n); $sizeGroups[$len] = $n + 1
                    }
                }
            } catch [System.UnauthorizedAccessException], [System.IO.IOException] {
                $totals.unreadable_dirs++
            }
            Add-Count $fanHist (Get-Log2Bucket $entries)
        }
        $totals.files += $row.files; $totals.bytes += $row.bytes; $totals.dirs += $row.dirs
        $rootRows += $row
    }

    # Same-size groups: an upper bound on byte-identical duplicates, path-based.
    $sharedFiles = 0L; $sharedBytes = 0L
    foreach ($kv in $sizeGroups.GetEnumerator()) {
        if ($kv.Value -lt 2) { continue }
        $sharedFiles += $kv.Value; $sharedBytes += $kv.Key * $kv.Value
    }

    # Extensions: top N with at least MinExtensionCount files; everything else folds into 'other'.
    $extensions = [ordered]@{}; $other = 0L
    foreach ($kv in ($extCount.GetEnumerator() | Sort-Object Value -Descending)) {
        if ($extensions.Count -lt $TopExtensions -and $kv.Value -ge $MinExtensionCount) {
            $extensions[$kv.Key] = $kv.Value
        } else { $other += $kv.Value }
    }
    $extensions['other'] = $other

    $sample = $null
    if ($SampleHash) {
        $plan = Select-HashBudget -SizeGroups $sizeGroups -MaxBytes $MaxHashBytes -MaxGroups $SampleGroups -Rng $Rng
        $msg = 'Hashing {0} of {1} same-size groups will read about {2:N2} GB (budget {3:N2} GB).'
        Write-Host ($msg -f $plan.Sizes.Count, $plan.GroupsEligible, ($plan.ProjectedBytes / 1GB), ($MaxHashBytes / 1GB))
        $measured = Measure-HashGroups -Members (Find-GroupMembers $Roots $plan.Sizes) -MaxBytes $MaxHashBytes
        $sample = [ordered]@{ max_hash_bytes = $MaxHashBytes; groups_eligible = $plan.GroupsEligible;
            groups_selected = $plan.Sizes.Count; projected_bytes = $plan.ProjectedBytes }
        foreach ($k in $measured.Keys) { $sample[$k] = $measured[$k] }
        $sample['note'] = $PathDuplicateNote
    }

    $sizeHist = [ordered]@{}
    foreach ($k in ($sizeFiles.Keys | Sort-Object)) { $sizeHist["$k"] = [ordered]@{ files = $sizeFiles[$k]; bytes = $sizeBytes[$k] } }

    $report = [ordered]@{
        schema = 'lel-drive-stats/2'
        date = (Get-Date).ToString('yyyy-MM-dd')
        roots = $rootRows
        totals = $totals
        size_log2_hist = $sizeHist
        depth_hist = ConvertTo-SortedMap $depthHist
        entries_per_dir_log2_hist = ConvertTo-SortedMap $fanHist
        extensions = $extensions
        shared_size = [ordered]@{ files = $sharedFiles; bytes = $sharedBytes; note = $PathDuplicateNote }
        sample_hash = $sample
    }
    Write-NewFile $Out ($report | ConvertTo-Json -Depth 6)
    Write-Host "Wrote $Out ($($totals.files) files, $([math]::Round($totals.bytes / 1TB, 2)) TB). Check it before sending."
}

# Dot-sourcing (the tests) loads the functions only.
if ($MyInvocation.InvocationName -ne '.') {
    Invoke-DriveStats -Roots $Roots -Out $Out -TopExtensions $TopExtensions -MinExtensionCount $MinExtensionCount `
        -SampleHash:$SampleHash -MaxHashBytes $MaxHashBytes -SampleGroups $SampleGroups
}
