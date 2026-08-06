<#
.SYNOPSIS
    Cut GitHub releases for the rusty_tls version tags, with bodies taken from CHANGELOG.md.

.DESCRIPTION
    Creates one GitHub release per entry in the $Releases table below.

    Every release is checked before it is created:

      * the tag must already exist on the remote (this script never creates tags --
        use Cut-RustyTlsTags.ps1 for that, and run it first);
      * the tag must be an annotated tag object, matching repo convention;
      * no release may already exist for that tag;
      * the CHANGELOG section named by the entry must exist and be non-empty,
        unless the entry sets Body explicitly.

    A release whose checks fail is reported and skipped. The rest still run, so a
    single bad entry does not block the batch.

.PARAMETER Repo
    owner/name. Defaults to baileyrd/rusty_tls.

.PARAMETER ChangelogPath
    Path to CHANGELOG.md. Defaults to ./CHANGELOG.md.

.PARAMETER Draft
    Create every release as a draft, so you can review the rendered bodies in the
    GitHub UI before publishing. Recommended for the first run.

.PARAMETER Latest
    Tag to mark as the "Latest" release. Defaults to the highest version in the
    table. Pass an empty string to let GitHub decide.

.EXAMPLE
    ./New-RustyTlsReleases.ps1 -WhatIf
    Show what would be created, without calling the API.

.EXAMPLE
    ./New-RustyTlsReleases.ps1 -Draft
    Create all eleven as drafts for review.

.NOTES
    Requires the gh CLI, authenticated with a token carrying `repo` scope
    (contents: write). Verify with:  gh auth status
#>

[CmdletBinding(SupportsShouldProcess)]
param(
    [string] $Repo          = 'baileyrd/rusty_tls',
    [string] $ChangelogPath = './CHANGELOG.md',
    [switch] $Draft,
    [string] $Latest        = 'v0.10.1'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# ---------------------------------------------------------------------------
# The releases to cut.
#
#   Tag       - the git tag. Must already exist on the remote.
#   Section   - the CHANGELOG.md version whose section becomes the release body,
#               i.e. the X.Y.Z in "## [X.Y.Z] - DATE". Usually Tag minus the "v".
#   Name      - release title. Defaults to the tag if omitted.
#   Body      - literal body, used INSTEAD of the CHANGELOG section. Only needed
#               where no section exists.
#
# v0.1.0 predates the changelog: CHANGELOG.md starts at 0.2.0, whose entry is
# "Adopt real version numbers, and stop the version field going stale" (#40).
# There is nothing to extract, so its body is written out here instead.
#
# There is deliberately NO v0.8.0 entry. CHANGELOG.md has a [0.8.0] section, but
# 0.8.0 was never a state of main - PR #57 carried two version bumps, so main
# went 0.7.0 -> 0.9.0 directly and no commit ever declared 0.8.0. There is no
# commit to point a release at. See the note at the end of this file.
# ---------------------------------------------------------------------------
$Releases = @(
    @{ Tag = 'v0.1.0';  Section = $null;   Name = 'v0.1.0';
       Body = @'
Initial library release: the sync TLS client, `TrustPolicy`, and hermetic tests.

This tag predates the changelog -- `CHANGELOG.md` begins at 0.2.0, the release
that adopted real version numbers. For what shipped here and in the releases
that followed it, see `RELEASE_NOTES.md`, which covers this period entry by
entry rather than by version.
'@ }
    @{ Tag = 'v0.2.0';  Section = '0.2.0'  }
    @{ Tag = 'v0.2.1';  Section = '0.2.1'  }
    @{ Tag = 'v0.3.0';  Section = '0.3.0'  }
    @{ Tag = 'v0.4.0';  Section = '0.4.0'  }
    @{ Tag = 'v0.5.0';  Section = '0.5.0'  }
    @{ Tag = 'v0.6.0';  Section = '0.6.0'  }
    @{ Tag = 'v0.7.0';  Section = '0.7.0'  }
    @{ Tag = 'v0.9.0';  Section = '0.9.0'  }
    @{ Tag = 'v0.10.0'; Section = '0.10.0' }
    @{ Tag = 'v0.10.1'; Section = '0.10.1' }
)

# ---------------------------------------------------------------------------
# Preconditions
# ---------------------------------------------------------------------------
if (-not (Get-Command gh -ErrorAction SilentlyContinue)) {
    throw 'gh CLI not found on PATH. Install it from https://cli.github.com and run: gh auth login'
}

gh auth status 2>&1 | Out-Null
if ($LASTEXITCODE -ne 0) {
    throw 'gh is installed but not authenticated. Run: gh auth login'
}

if (-not (Test-Path -LiteralPath $ChangelogPath)) {
    throw "Changelog not found at '$ChangelogPath'. Run this from the repo root, or pass -ChangelogPath."
}
$ChangelogPath = (Resolve-Path -LiteralPath $ChangelogPath).Path
Write-Verbose "Reading changelog from $ChangelogPath"

# ---------------------------------------------------------------------------
# Pull one "## [X.Y.Z] - DATE" section out of the changelog.
#
# Returns the section body with the header line removed, trailing HTML comments
# stripped (the file ends with a commented-out 0.1.0 placeholder that would
# otherwise be swept into the last section), and surrounding blank lines trimmed.
# Returns $null when the section does not exist or is empty.
# ---------------------------------------------------------------------------
function Get-ChangelogSection {
    [CmdletBinding()]
    param(
        # AllowEmptyString is required, not decorative: a Mandatory [string[]]
        # rejects any array containing an empty element, and a changelog is
        # mostly blank lines. Without this the first call fails outright.
        [Parameter(Mandatory)] [AllowEmptyString()] [string[]] $Lines,
        [Parameter(Mandatory)] [string]                        $Version
    )

    $escaped = [regex]::Escape($Version)
    $start   = $null

    for ($i = 0; $i -lt $Lines.Count; $i++) {
        if ($Lines[$i] -match "^##\s+\[$escaped\]") { $start = $i; break }
    }
    if ($null -eq $start) { return $null }

    $end = $Lines.Count
    for ($i = $start + 1; $i -lt $Lines.Count; $i++) {
        if ($Lines[$i] -match '^##\s') { $end = $i; break }
    }

    $body = $Lines[($start + 1)..($end - 1)]

    # Drop a trailing HTML comment block, and anything after it.
    for ($i = 0; $i -lt $body.Count; $i++) {
        if ($body[$i] -match '^\s*<!--') {
            if ($i -eq 0) { return $null }
            $body = $body[0..($i - 1)]
            break
        }
    }

    $text = ($body -join "`n").Trim()
    if ([string]::IsNullOrWhiteSpace($text)) { return $null }
    return $text
}

# ---------------------------------------------------------------------------
# Remote state, fetched once.
# ---------------------------------------------------------------------------
Write-Host "Repository: $Repo" -ForegroundColor Cyan

$existingReleaseTags = @{}
$releasesJson = gh api "repos/$Repo/releases?per_page=100" --paginate 2>&1
if ($LASTEXITCODE -ne 0) {
    throw "Could not list releases for $Repo. gh said: $releasesJson"
}
foreach ($r in ($releasesJson | ConvertFrom-Json)) {
    $existingReleaseTags[$r.tag_name] = $r.html_url
}
Write-Host ("Existing releases: {0}" -f $(if ($existingReleaseTags.Count) { ($existingReleaseTags.Keys | Sort-Object) -join ', ' } else { '(none)' }))

$changelogLines = Get-Content -LiteralPath $ChangelogPath

# ---------------------------------------------------------------------------
# Verify every entry before creating anything.
# ---------------------------------------------------------------------------
$plan = foreach ($entry in $Releases) {

    $tag      = $entry.Tag
    $problems = [System.Collections.Generic.List[string]]::new()
    $body     = $null

    # -- the tag must exist on the remote, and be annotated ------------------
    $refJson = gh api "repos/$Repo/git/ref/tags/$tag" 2>&1
    if ($LASTEXITCODE -ne 0) {
        $problems.Add('tag does not exist on the remote (run the tagging script first)')
        $commit = ''
    }
    else {
        $ref = $refJson | ConvertFrom-Json
        if ($ref.object.type -ne 'tag') {
            $problems.Add("tag is $($ref.object.type), not an annotated tag object")
            $commit = $ref.object.sha
        }
        else {
            $tagObj = gh api "repos/$Repo/git/tags/$($ref.object.sha)" 2>&1 | ConvertFrom-Json
            $commit = $tagObj.object.sha
        }
    }

    # -- no release may already exist ---------------------------------------
    if ($existingReleaseTags.ContainsKey($tag)) {
        $problems.Add("a release already exists: $($existingReleaseTags[$tag])")
    }

    # -- the body must resolve ----------------------------------------------
    if ($entry.ContainsKey('Body') -and $entry.Body) {
        $body = $entry.Body.Trim()
    }
    elseif ($entry.Section) {
        $body = Get-ChangelogSection -Lines $changelogLines -Version $entry.Section
        if (-not $body) {
            $problems.Add("no non-empty [$($entry.Section)] section in $(Split-Path -Leaf $ChangelogPath)")
        }
    }
    else {
        $problems.Add('entry has neither Section nor Body')
    }

    [pscustomobject]@{
        Tag      = $tag
        Name     = $(if ($entry.ContainsKey('Name') -and $entry.Name) { $entry.Name } else { $tag })
        Commit   = $(if ($commit) { $commit.Substring(0, [Math]::Min(7, $commit.Length)) } else { '-' })
        Bytes    = $(if ($body) { $body.Length } else { 0 })
        Status   = $(if ($problems.Count) { 'SKIP' } else { 'ready' })
        Problems = ($problems -join '; ')
        Body     = $body
    }
}

# Out-String, not Format-Table alone: a redirected or CI host reports
# BufferSize.Width = -1 and Format-Table then emits nothing at all - the table
# would vanish in exactly the runs where it is captured for the record.
$plan |
    Format-Table Tag, Name, Commit, Bytes, Status, Problems -AutoSize |
    Out-String -Width 200 |
    Write-Host

$ready   = @($plan | Where-Object Status -eq 'ready')
$skipped = @($plan | Where-Object Status -eq 'SKIP')

if ($skipped.Count) {
    Write-Warning "$($skipped.Count) entr$(if ($skipped.Count -eq 1) { 'y' } else { 'ies' }) will be skipped:"
    foreach ($s in $skipped) { Write-Warning "  $($s.Tag): $($s.Problems)" }
}

if (-not $ready.Count) {
    Write-Host 'Nothing to create.' -ForegroundColor Yellow
    return
}

# ---------------------------------------------------------------------------
# Create.
# ---------------------------------------------------------------------------
$created = 0
$failed  = 0

foreach ($r in $ready) {

    $isLatest = ($Latest -and $r.Tag -eq $Latest)
    $label    = "release $($r.Tag)$(if ($Draft) { ' (draft)' })$(if ($isLatest) { ' [latest]' })"

    if (-not $PSCmdlet.ShouldProcess($Repo, "Create $label")) { continue }

    # Body goes through a temp file: release notes contain backticks, quotes and
    # newlines that do not survive being spliced onto a command line.
    $bodyFile = [System.IO.Path]::GetTempFileName()
    try {
        Set-Content -LiteralPath $bodyFile -Value $r.Body -Encoding utf8NoBOM

        $ghArgs = @(
            'api', "repos/$Repo/releases",
            '--method', 'POST',
            '-f', "tag_name=$($r.Tag)",
            '-f', "name=$($r.Name)",
            '-F', "body=@$bodyFile",
            '-F', "draft=$(if ($Draft) { 'true' } else { 'false' })",
            '-F', 'prerelease=false',
            '-f', "make_latest=$(if ($isLatest) { 'true' } else { 'false' })"
        )

        $out = gh @ghArgs 2>&1
        if ($LASTEXITCODE -ne 0) {
            Write-Warning "FAILED $($r.Tag): $out"
            $failed++
        }
        else {
            $url = ($out | ConvertFrom-Json).html_url
            Write-Host "created $($r.Tag)  $url" -ForegroundColor Green
            $created++
        }
    }
    finally {
        Remove-Item -LiteralPath $bodyFile -Force -ErrorAction SilentlyContinue
    }
}

Write-Host ''
Write-Host "created=$created failed=$failed skipped=$($skipped.Count)" -ForegroundColor Cyan
if ($failed) { exit 1 }

# ---------------------------------------------------------------------------
# On 0.8.0
#
# CHANGELOG.md carries a [0.8.0] section and RELEASE_NOTES.md a v0.8.0 entry,
# but no commit on main ever declared version 0.8.0 and there is no v0.8.0 tag.
# PR #57 carried two bumps in one merge, so main went 0.7.0 (b8276fe) straight
# to 0.9.0 (5a5cf6b).
#
# A release needs a commit to point at, and 0.8.0 has none, so it is left out
# rather than pointed at a neighbour - which would attach notes to a tree that
# never declared that version. Two ways to settle it, both outside this script:
#
#   * fold the [0.8.0] section into [0.9.0] in CHANGELOG.md, since that is the
#     release the work actually shipped in; or
#   * leave both files as they are, treating 0.8.0 as a development milestone
#     that is documented but was never released.
#
# The second needs no code change and is the honest description of what
# happened. Either way it is a judgement call about the changelog, not
# something this script should decide.
# ---------------------------------------------------------------------------
