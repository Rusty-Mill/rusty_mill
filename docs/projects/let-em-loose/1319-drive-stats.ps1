#Requires -Version 7.0
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
  printed and reported, and every read is checked again against the budget in case files appeared or
  changed size between the passes. Hashes stay in memory; only counts are written. Nothing on the
  scanned roots is modified.

  The duplicate figures are path-based: two hardlinks to one file count as two duplicate paths though
  they share storage, so they are not reclaimable space.
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

function Select-HashBudget {
    <#
      Pure: picks whole same-size groups at random until the byte budget is used.
      $SizeGroups maps file size to the number of files of that size (first-pass metadata).
      A group costs size x count; groups over the remaining budget are skipped, smaller ones still fit.
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
        $cost = $size * $SizeGroups[$size]
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

function Measure-HashGroups {
    <#
      Hashes each group whole, never past the byte budget. A group whose current cost (size x members
      found now) no longer fits is skipped; a file whose size changed since it was found is not read.
      Returns counts only.
    #>
    param(
        [hashtable]$Members,
        [long]$MaxBytes,
        [scriptblock]$Hasher = { param([string]$p) (Get-FileHash -LiteralPath $p -Algorithm SHA256).Hash }
    )
    $stats = [ordered]@{ groups_hashed = 0; groups_skipped_budget = 0; files_hashed = 0L; files_changed = 0L;
        bytes_read = 0L; path_duplicate_files = 0L; path_duplicate_bytes = 0L }
    foreach ($size in ($Members.Keys | Sort-Object)) {
        $paths = $Members[$size]
        if ($paths.Count -lt 2) { continue }
        if ($stats.bytes_read + $size * $paths.Count -gt $MaxBytes) { $stats.groups_skipped_budget++; continue }
        $byHash = @{}
        foreach ($path in $paths) {
            $now = [System.IO.FileInfo]::new($path)
            if (-not $now.Exists -or $now.Length -ne $size) { $stats.files_changed++; continue }
            try { $h = & $Hasher $path } catch { $stats.files_changed++; continue }
            $stats.files_hashed++; $stats.bytes_read += $size
            Add-Count $byHash $h
        }
        $stats.groups_hashed++
        foreach ($count in $byHash.Values) {
            if ($count -lt 2) { continue }
            $stats.path_duplicate_files += $count; $stats.path_duplicate_bytes += $size * ($count - 1)
        }
    }
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
    $report | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $Out -Encoding utf8
    Write-Host "Wrote $Out ($($totals.files) files, $([math]::Round($totals.bytes / 1TB, 2)) TB). Check it before sending."
}

# Dot-sourcing (the tests) loads the functions only.
if ($MyInvocation.InvocationName -ne '.') {
    Invoke-DriveStats -Roots $Roots -Out $Out -TopExtensions $TopExtensions -MinExtensionCount $MinExtensionCount `
        -SampleHash:$SampleHash -MaxHashBytes $MaxHashBytes -SampleGroups $SampleGroups
}
