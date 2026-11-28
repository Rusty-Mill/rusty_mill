#Requires -Version 7.0
<#
.SYNOPSIS
  Read-only aggregate shape of the given roots, for Let Em Loose card #1319.
.DESCRIPTION
  Walks each root without following reparse points (symlinks, junctions) and writes one JSON file of
  counts and histograms. It writes no file names, paths or hashes (D231). With -SampleHash it hashes
  files in a random sample of same-size groups to estimate the byte-identical duplicate rate; the
  hashes stay in memory and only counts are written. Nothing on the scanned roots is modified.
.EXAMPLE
  pwsh -File .\1319-drive-stats.ps1 -Roots 'D:\','\\nas\share' -Out "$HOME\lel-drive-stats.json"
#>
param(
    [Parameter(Mandatory)][string[]]$Roots,
    [string]$Out = (Join-Path $HOME 'lel-drive-stats.json'),
    [int]$TopExtensions = 25,
    [int]$MinExtensionCount = 10,
    [switch]$SampleHash,
    [int]$SampleGroups = 2000
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

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

# Same-size groups: an upper bound on byte-identical duplicates.
$sharedFiles = 0L; $sharedBytes = 0L; $sharedSizes = [System.Collections.Generic.List[long]]::new()
foreach ($kv in $sizeGroups.GetEnumerator()) {
    if ($kv.Value -lt 2) { continue }
    $sharedFiles += $kv.Value; $sharedBytes += $kv.Key * $kv.Value; $sharedSizes.Add($kv.Key)
}

# Extensions: top N with at least MinExtensionCount files; everything else folds into 'other'.
$extensions = [ordered]@{}; $other = 0L
$ranked = $extCount.GetEnumerator() | Sort-Object Value -Descending
foreach ($kv in $ranked) {
    if ($extensions.Count -lt $TopExtensions -and $kv.Value -ge $MinExtensionCount) {
        $extensions[$kv.Key] = $kv.Value
    } else { $other += $kv.Value }
}
$extensions['other'] = $other

$sample = $null
if ($SampleHash -and $sharedSizes.Count -gt 0) {
    # Second pass: collect files whose size is in the sample, hash them, keep only counts.
    $chosen = [System.Collections.Generic.HashSet[long]]::new()
    $sharedSizes | Get-Random -Count ([math]::Min($SampleGroups, $sharedSizes.Count)) | ForEach-Object { [void]$chosen.Add($_) }
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
                    if (-not $chosen.Contains($e.Length)) { continue }
                    if (-not $members.ContainsKey($e.Length)) { $members[$e.Length] = [System.Collections.Generic.List[string]]::new() }
                    $members[$e.Length].Add($e.FullName)
                }
            } catch [System.UnauthorizedAccessException], [System.IO.IOException] { }
        }
    }
    $filesHashed = 0L; $dupFiles = 0L; $redundantBytes = 0L; $sampleBytes = 0L
    foreach ($size in $members.Keys) {
        $byHash = @{}
        foreach ($path in $members[$size]) {
            try { $h = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash } catch { continue }
            $filesHashed++; $sampleBytes += $size
            Add-Count $byHash $h
        }
        foreach ($count in $byHash.Values) {
            if ($count -lt 2) { continue }
            $dupFiles += $count; $redundantBytes += $size * ($count - 1)
        }
    }
    $sample = [ordered]@{ groups = $members.Count; files_hashed = $filesHashed; bytes_hashed = $sampleBytes;
        files_with_identical_sibling = $dupFiles; redundant_bytes = $redundantBytes }
}

function ConvertTo-SortedMap([hashtable]$h) {
    $m = [ordered]@{}
    foreach ($k in ($h.Keys | Sort-Object { [long]$_ })) { $m["$k"] = $h[$k] }
    return $m
}

$sizeHist = [ordered]@{}
foreach ($k in ($sizeFiles.Keys | Sort-Object)) { $sizeHist["$k"] = [ordered]@{ files = $sizeFiles[$k]; bytes = $sizeBytes[$k] } }

$report = [ordered]@{
    schema = 'lel-drive-stats/1'
    date = (Get-Date).ToString('yyyy-MM-dd')
    roots = $rootRows
    totals = $totals
    size_log2_hist = $sizeHist
    depth_hist = ConvertTo-SortedMap $depthHist
    entries_per_dir_log2_hist = ConvertTo-SortedMap $fanHist
    extensions = $extensions
    shared_size = [ordered]@{ files = $sharedFiles; bytes = $sharedBytes }
    sample_hash = $sample
}
$report | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $Out -Encoding utf8
Write-Host "Wrote $Out ($($totals.files) files, $([math]::Round($totals.bytes / 1TB, 2)) TB). Check it before sending."
