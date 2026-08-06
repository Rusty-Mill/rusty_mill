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

.PARAMETER PublishDrafts
    Publish the existing DRAFT releases instead of creating anything. This is
    the other half of -Draft: draft first, read the rendered bodies in the UI,
    then publish with this.

    Only drafts whose tag appears in $Releases below are touched. A draft for
    some other tag is left alone rather than swept up, because this script's
    table is the statement of what it is responsible for.

    Creates nothing. A tag with no release at all is reported, not created --
    use a normal run for that.

.PARAMETER Latest
    Tag to mark as the "Latest" release. Defaults to the highest version in the
    table. Pass an empty string to let GitHub decide.

.EXAMPLE
    ./New-RustyTlsReleases.ps1 -WhatIf
    Show what would be created, without calling the API.

.EXAMPLE
    ./New-RustyTlsReleases.ps1 -Draft
    Create all eleven as drafts for review.

.EXAMPLE
    ./New-RustyTlsReleases.ps1 -PublishDrafts -WhatIf
    Show which drafts would be published, without calling the API.

.EXAMPLE
    ./New-RustyTlsReleases.ps1 -PublishDrafts
    Publish them, oldest first, leaving $Latest as the "Latest" release.

.NOTES
    Requires the gh CLI, authenticated with a token carrying `repo` scope
    (contents: write).

    Do NOT verify that with `gh auth status` -- it exits 0 even when it prints
    "The token in GH_TOKEN is invalid." This script probes with a real request
    instead, for that reason.
#>

[CmdletBinding(SupportsShouldProcess)]
param(
    [string] $Repo          = 'baileyrd/rusty_tls',
    [string] $ChangelogPath = './CHANGELOG.md',
    [switch] $Draft,
    [switch] $PublishDrafts,
    [string] $Latest        = 'v0.10.1'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# Argument validation first, before anything reaches the network. A run that
# cannot be correct should say so immediately rather than after two API calls.
if ($Draft -and $PublishDrafts) {
    throw '-Draft and -PublishDrafts are opposites: one creates releases held back from publication, the other releases what is held back. Pick one.'
}

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

# Do NOT gate this on `gh auth status`, which exits 0 even when it prints
# "The token in GH_TOKEN is invalid." -- measured, not assumed. An exit code
# that reports success while the body reports failure makes the check
# decorative, and this one guards every call below it.
#
# The only trustworthy test is a real authenticated request, so make one. This
# also catches the cases `auth status` cannot know about: a token whose scopes
# are too narrow, and a network path that intercepts api.github.com and answers
# 403 on its behalf.
$probe = gh api "repos/$Repo" --jq '.full_name' 2>&1
if ($LASTEXITCODE -ne 0) {
    throw "Cannot reach $Repo through gh. Check `gh auth status`, that the token carries ``repo`` scope, and that nothing is intercepting api.github.com. gh said: $probe"
}
Write-Verbose "Authenticated; $probe is readable."

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

# Keyed by tag, holding the whole release rather than just its URL: publishing
# needs the id, and deciding whether to publish needs the draft flag. A draft
# IS returned by this endpoint for an authorised caller, which is what makes a
# normal run skip a tag that is only drafted rather than creating a duplicate.
$existingReleases = @{}
$releasesJson = gh api "repos/$Repo/releases?per_page=100" --paginate 2>&1
if ($LASTEXITCODE -ne 0) {
    throw "Could not list releases for $Repo. gh said: $releasesJson"
}
foreach ($r in ($releasesJson | ConvertFrom-Json)) {
    $existingReleases[$r.tag_name] = [pscustomobject]@{
        Id = $r.id; Draft = $r.draft; Url = $r.html_url
    }
}
Write-Host ("Existing releases: {0}" -f $(if ($existingReleases.Count) {
    (($existingReleases.Keys | Sort-Object) | ForEach-Object {
        "$_$(if ($existingReleases[$_].Draft) { ' (draft)' })" }) -join ', '
} else { '(none)' }))

# ---------------------------------------------------------------------------
# -PublishDrafts: flip existing drafts to published. Creates nothing, reads no
# changelog, and returns before the creation path below.
#
# Publishing is a PATCH, not a POST -- a draft is an existing release with
# draft=true, so publishing it is an edit. That also makes a second run
# harmless: patching a published release to draft=false leaves it as it is.
# ---------------------------------------------------------------------------
if ($PublishDrafts) {

    # Ascending version order matters, and is not the table's order to assume:
    # "Latest" is whatever was published most recently unless make_latest says
    # otherwise, so publishing newest-first would leave the oldest release
    # sitting as Latest for anyone who looked in between. Sorted here rather
    # than relying on how $Releases happens to be written.
    $ordered = $Releases |
        Sort-Object @{ Expression = { [version] ($_.Tag.TrimStart('v')) } }

    $draftPlan = foreach ($entry in $ordered) {
        $tag  = $entry.Tag
        $have = $existingReleases[$tag]
        [pscustomobject]@{
            Tag      = $tag
            Id       = $(if ($have) { $have.Id } else { $null })
            Action   = $(if (-not $have)      { 'no release (a normal run creates it)' }
                         elseif (-not $have.Draft) { 'already published' }
                         else                 { 'publish' })
            Latest   = $($Latest -and $tag -eq $Latest)
        }
    }

    $draftPlan |
        Format-Table Tag, Id, Action, Latest -AutoSize |
        Out-String -Width 200 |
        Write-Host

    $toPublish = @($draftPlan | Where-Object Action -eq 'publish')

    # Say nothing-to-do out loud rather than reporting a silent success. Zero
    # drafts is a legitimate state, but it is indistinguishable from "the tag
    # filter matched nothing" unless the count is stated.
    if (-not $toPublish.Count) {
        Write-Host "No drafts to publish. ($($draftPlan.Count) entries checked.)" -ForegroundColor Yellow
        return
    }

    $published = 0
    $failed    = 0
    foreach ($d in $toPublish) {
        $label = "release $($d.Tag)$(if ($d.Latest) { ' [latest]' })"
        if (-not $PSCmdlet.ShouldProcess($Repo, "Publish $label")) { continue }

        $out = gh api "repos/$Repo/releases/$($d.Id)" --method PATCH `
                  -F 'draft=false' -f "make_latest=$(if ($d.Latest) { 'true' } else { 'false' })" 2>&1
        if ($LASTEXITCODE -ne 0) {
            Write-Warning "FAILED $($d.Tag): $out"
            $failed++
            continue
        }

        # A draft's URL is .../releases/tag/untagged-<hash> and becomes
        # .../releases/tag/<tag> once published, so the URL is the receipt.
        # Check it rather than trusting the exit code.
        $url = ($out | ConvertFrom-Json).html_url
        if ($url -match 'untagged-') {
            Write-Warning "$($d.Tag): API reported success but the URL is still a draft URL ($url)"
            $failed++
        }
        else {
            Write-Host "published $($d.Tag)  $url" -ForegroundColor Green
            $published++
        }
    }

    Write-Host ''
    Write-Host "published=$published failed=$failed" -ForegroundColor Cyan
    if ($failed) { exit 1 }
    return
}

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
    if ($existingReleases.ContainsKey($tag)) {
        $have = $existingReleases[$tag]
        $problems.Add("a release already exists$(if ($have.Draft) { ' as a draft -- publish it with -PublishDrafts' }): $($have.Url)")
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
