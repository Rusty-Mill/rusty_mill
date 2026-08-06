#Requires -Version 5.1
<#
.SYNOPSIS
    Cut the annotated release tags rusty_tls is missing, using the gh CLI.

.DESCRIPTION
    docs/versioning.md, "Cutting a tag":

        1. The version bump lands on main as part of an ordinary PR.
        2. The tag is cut ON THE MERGE COMMIT, not on the branch that produced it.
        3. git tag -a vX.Y.Z -m "..." -- ANNOTATED, so it carries a date and an author.
        4. Push the tag explicitly.

    Rule 3 is why this does not use `gh release create`: that creates a
    LIGHTWEIGHT tag when the tag does not already exist. This uses the Git Data
    API instead (POST git/tags then POST git/refs), which produces a real
    annotated tag object -- the same kind as the three tags already on the repo.

    Rule 2 is why $Tags below pins merge commits. Before creating anything, each
    pin is VERIFIED: the script fetches Cargo.toml at that exact commit and
    requires it to declare the version being tagged. A pin that does not match
    is reported and skipped rather than tagged wrongly.

.PARAMETER Repo
    owner/name. Defaults to baileyrd/rusty_tls.

.PARAMETER SkipVerification
    Create tags without checking Cargo.toml at each pin. Not recommended --
    the verification is the entire reason this script is longer than four lines.

.EXAMPLE
    .\Cut-RustyTlsTags.ps1 -WhatIf
    Report what would happen. Verification still runs. Nothing is created.

.EXAMPLE
    .\Cut-RustyTlsTags.ps1
    Verify, then create the missing tags, prompting per tag.

.EXAMPLE
    .\Cut-RustyTlsTags.ps1 -Confirm:$false
    Everything, no prompts.

.NOTES
    Requires: gh (authenticated, with `repo` scope). No local clone needed.

    Every version in $Tags is currently tagged on the remote, so a run today
    reports "skip (exists)" for all of them and creates nothing. That is the
    expected steady state, not a failure -- the script stays useful because the
    next release adds one row and runs it again. Verification still runs on
    every entry, so a run is also a cheap re-check that no published tag has
    drifted from the commit whose Cargo.toml declares its version.
#>
# Write-Host is deliberate: this is an interactive report whose colour and
# layout are the point, not data for a pipeline. ShouldProcess lives on the
# caller of New-AnnotatedTag, which is the only thing that mutates anything.
[Diagnostics.CodeAnalysis.SuppressMessageAttribute('PSAvoidUsingWriteHost', '')]
[Diagnostics.CodeAnalysis.SuppressMessageAttribute('PSUseShouldProcessForStateChangingFunctions', '')]
[Diagnostics.CodeAnalysis.SuppressMessageAttribute('PSUseSingularNouns', '')]
[CmdletBinding(SupportsShouldProcess = $true, ConfirmImpact = 'Medium')]
param(
    [ValidatePattern('^[^/]+/[^/]+$')]
    [string] $Repo = 'baileyrd/rusty_tls',

    [switch] $SkipVerification
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# ---------------------------------------------------------------------------
# The pins. Edit Version/Commit here; everything below verifies rather than
# trusts them.
#
# Each Commit is the merge commit at which `main` FIRST declared that version,
# which is what docs/versioning.md rule 2 asks for. The two tags already cut
# this way (v0.4.0 -> 174fcd2, v0.7.0 -> 07fee87) both follow it, so this is
# the established convention and not an invention.
#
# Message defaults to the version string, matching the three existing tags,
# whose subjects are exactly "v0.2.1", "v0.4.0", "v0.7.0".
# ---------------------------------------------------------------------------
$Tags = @(
    [pscustomobject]@{ Version = 'v0.1.0';  Commit = 'd138414'; Message = $null; Why = 'squash-merge of #3 - see the single-parent note below' }
    [pscustomobject]@{ Version = 'v0.2.0';  Commit = 'ab1a4d5'; Message = $null; Why = 'merge of #40 - the release that adopted real version numbers' }
    [pscustomobject]@{ Version = 'v0.2.1';  Commit = '385d0cb'; Message = $null; Why = 'merge of #48 - see the mid-run note below' }
    [pscustomobject]@{ Version = 'v0.3.0';  Commit = 'e463711'; Message = $null; Why = 'merge of #51' }
    [pscustomobject]@{ Version = 'v0.4.0';  Commit = '174fcd2'; Message = $null; Why = 'merge of #52' }
    [pscustomobject]@{ Version = 'v0.5.0';  Commit = '35b3904'; Message = $null; Why = 'merge of #53' }
    [pscustomobject]@{ Version = 'v0.6.0';  Commit = 'd96c8de'; Message = $null; Why = 'merge of #54' }
    [pscustomobject]@{ Version = 'v0.7.0';  Commit = '07fee87'; Message = $null; Why = 'merge of #55' }
    [pscustomobject]@{ Version = 'v0.9.0';  Commit = '5a5cf6b'; Message = $null; Why = 'merge of #57 - main became 0.9.0 here' }
    [pscustomobject]@{ Version = 'v0.10.0'; Commit = '48d6f89'; Message = $null; Why = 'merge of #60 - main became 0.10.0 here' }
    [pscustomobject]@{ Version = 'v0.10.1'; Commit = '00f7a1f'; Message = $null; Why = 'merge of #61 - main became 0.10.1 here' }

    # --- Two entries above will report a verification warning. Both are ------
    # --- correct as pinned; the warnings are accurate, not stale. ------------
    #
    # v0.1.0 -> d138414 is the ONLY pin here that is not a merge commit. PR #3
    # was squash-merged, so the commit has one parent and the rule-2 check
    # warns. The version check still passes: Cargo.toml at d138414 says 0.1.0,
    # and it is the first commit on main to say anything at all. Tagging it is
    # right; the warning is the script correctly reporting an exception.
    #
    # v0.2.1 -> 385d0cb is the SECOND of four commits on main declaring 0.2.1,
    # not the first (0de6b59). Every other pin here is a bump commit. This one
    # is pinned to match the tag that already exists rather than to the
    # convention, because moving a published tag to fix a cosmetic
    # inconsistency is worse than the inconsistency.
    #
    # --- v0.8.0 is deliberately absent, and this is now settled. -------------
    #
    # `main` NEVER declared 0.8.0. PR #57 carried two version bumps in one
    # branch, so main went 0.7.0 (b8276fe) straight to 0.9.0 (5a5cf6b). The
    # only commit where Cargo.toml says 0.8.0 is c7d8428, a BRANCH commit that
    # the merge brought into history -- exactly what rule 2 says not to tag.
    #
    # An earlier version of this file laid out three options and picked none.
    # rusty_tls#64 picked one: leave v0.8.0 uncut, because it was never a state
    # of main, and fold the 0.8.0 entry in CHANGELOG.md and RELEASE_NOTES.md
    # into 0.9.0, which is the release the work actually shipped in. There is
    # now no document naming a 0.8.0 and no tag for one, which is consistent.
    #
    # Do not "fix" the gap in the tag sequence by adding one. The gap is the
    # accurate record.
)

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

function Invoke-Gh {
    param([string[]] $Arguments, [switch] $AllowFailure)
    $out = & gh @Arguments 2>&1
    $code = $LASTEXITCODE
    if ($code -ne 0 -and -not $AllowFailure) {
        throw "gh $($Arguments -join ' ') failed ($code): $out"
    }
    [pscustomobject]@{ ExitCode = $code; Output = ($out | Out-String).Trim() }
}

function Assert-Prerequisites {
    if (-not (Get-Command gh -ErrorAction SilentlyContinue)) {
        throw "gh CLI not found on PATH. https://cli.github.com/"
    }
    # `gh auth status` is NOT checked here, deliberately: it exits 0 even when
    # it prints "The token in GH_TOKEN is invalid." -- measured, not assumed.
    # An exit code that says success while the body says failure is worse than
    # no check, so the probe below is the whole test. It also covers what
    # `auth status` cannot know: too-narrow scopes, and a network path that
    # intercepts api.github.com and answers 403 on its behalf.
    $probe = Invoke-Gh @('api', "repos/$Repo", '--jq', '.full_name') -AllowFailure
    if ($probe.ExitCode -ne 0) {
        throw "Cannot reach $Repo through gh. Check ``gh auth status``, that the token carries ``repo`` scope, and that nothing is intercepting api.github.com. gh said: $($probe.Output)"
    }
    Write-Verbose "Authenticated, and $Repo is readable."
}

function Get-ExistingTag {
    param([string] $Tag)
    $r = Invoke-Gh @('api', "repos/$Repo/git/ref/tags/$Tag", '--jq', '.object.sha') -AllowFailure
    if ($r.ExitCode -ne 0) { return $null }
    return $r.Output
}

function Resolve-Commit {
    param([string] $Ref)
    $r = Invoke-Gh @('api', "repos/$Repo/commits/$Ref", '--jq', '{sha:.sha,parents:(.parents|length),subject:(.commit.message|split("\n")[0])}') -AllowFailure
    if ($r.ExitCode -ne 0) { return $null }
    return ($r.Output | ConvertFrom-Json)
}

function Get-DeclaredVersion {
    # Reads Cargo.toml AT THAT COMMIT. This is the check that makes a pin
    # verifiable instead of asserted.
    param([string] $Sha)
    $r = Invoke-Gh @('api', "repos/$Repo/contents/Cargo.toml?ref=$Sha", '-H', 'Accept: application/vnd.github.raw') -AllowFailure
    if ($r.ExitCode -ne 0) { return $null }
    foreach ($line in ($r.Output -split "`n")) {
        if ($line -match '^\s*version\s*=\s*"([^"]+)"') { return $Matches[1] }
    }
    return $null
}

function Test-OnDefaultBranch {
    param([string] $Sha)
    $branch = (Invoke-Gh @('api', "repos/$Repo", '--jq', '.default_branch')).Output
    $r = Invoke-Gh @('api', "repos/$Repo/compare/$Sha...$branch", '--jq', '.status') -AllowFailure
    if ($r.ExitCode -ne 0) { return $false }
    return ($r.Output -in @('ahead', 'identical'))
}

function New-AnnotatedTag {
    # Git Data API, in two steps, because that is what produces an annotated
    # tag. `gh release create` would produce a lightweight one.
    param([string] $Tag, [string] $Sha, [string] $Message)

    $obj = Invoke-Gh @(
        'api', '--method', 'POST', "repos/$Repo/git/tags",
        '-f', "tag=$Tag", '-f', "message=$Message",
        '-f', "object=$Sha", '-f', 'type=commit',
        '--jq', '.sha'
    )
    $tagObject = $obj.Output
    Write-Verbose "Tag object $tagObject created for $Tag."

    Invoke-Gh @(
        'api', '--method', 'POST', "repos/$Repo/git/refs",
        '-f', "ref=refs/tags/$Tag", '-f', "sha=$tagObject"
    ) | Out-Null

    return $tagObject
}

# ---------------------------------------------------------------------------
# Verify everything before creating anything
# ---------------------------------------------------------------------------

Assert-Prerequisites

$selected = $Tags
if (-not $selected) { Write-Host "Nothing selected."; return }

$plan = foreach ($t in $selected) {
    $problems = New-Object System.Collections.Generic.List[string]

    $existing = Get-ExistingTag -Tag $t.Version
    $commit   = Resolve-Commit -Ref $t.Commit

    if (-not $commit) {
        $problems.Add("commit $($t.Commit) not found in $Repo")
        $declared = $null
    }
    else {
        if (-not $SkipVerification) {
            $declared = Get-DeclaredVersion -Sha $commit.sha
            $want     = $t.Version.TrimStart('v')
            if (-not $declared)          { $problems.Add("could not read Cargo.toml at $($t.Commit)") }
            elseif ($declared -ne $want) { $problems.Add("Cargo.toml at $($t.Commit) says $declared, not $want") }

            if ($commit.parents -lt 2)   { $problems.Add("not a merge commit ($($commit.parents) parent(s)) - versioning.md rule 2") }
            if (-not (Test-OnDefaultBranch -Sha $commit.sha)) { $problems.Add("not an ancestor of the default branch") }
        }
        else { $declared = '(unverified)' }
    }

    [pscustomobject]@{
        Version  = $t.Version
        Commit   = if ($commit) { $commit.sha.Substring(0,7) } else { $t.Commit }
        Declares = $declared
        Existing = if ($existing) { $existing.Substring(0,7) } else { '-' }
        Action   = if ($existing) { 'skip (exists)' }
                   elseif ($problems.Count) { 'BLOCKED' }
                   else { 'create' }
        Problems = ($problems -join '; ')
        Message  = if ($t.Message) { $t.Message } else { $t.Version }
        Sha      = if ($commit) { $commit.sha } else { $null }
        Why      = $t.Why
    }
}

Write-Host ""
Write-Host "Plan for $Repo" -ForegroundColor Cyan
# Rendered through Out-String with an explicit width, NOT `Format-Table
# -AutoSize`. -AutoSize emits nothing at all when the host reports no console
# width (BufferSize.Width = -1), which is every redirected or piped run -- so
# the verification table would vanish exactly when someone captured the output.
Write-Host (($plan |
    Format-Table Version, Commit, Declares, Existing, Action, Problems -Wrap |
    Out-String -Width 200).TrimEnd())

$blocked = @($plan | Where-Object Action -eq 'BLOCKED')
if ($blocked) {
    Write-Warning "$($blocked.Count) tag(s) blocked by verification and will NOT be created:"
    foreach ($b in $blocked) { Write-Warning "  $($b.Version): $($b.Problems)" }
}

$todo = @($plan | Where-Object Action -eq 'create')
if (-not $todo) {
    Write-Host "Nothing to create." -ForegroundColor Yellow
    return
}

# ---------------------------------------------------------------------------
# Create
# ---------------------------------------------------------------------------

foreach ($t in $todo) {
    $target = "$($t.Version) -> $($t.Commit) ($($t.Why))"
    if ($PSCmdlet.ShouldProcess($target, 'Create annotated tag')) {
        try {
            $obj = New-AnnotatedTag -Tag $t.Version -Sha $t.Sha -Message $t.Message
            Write-Host "  created $($t.Version) -> $($t.Commit)  [tag object $($obj.Substring(0,7))]" -ForegroundColor Green
        }
        catch {
            Write-Error "  FAILED $($t.Version): $_"
        }
    }
}

# ---------------------------------------------------------------------------
# Confirm what is actually on the remote now
# ---------------------------------------------------------------------------

Write-Host ""
Write-Host "Tags on $Repo now:" -ForegroundColor Cyan
$refs = (Invoke-Gh @('api', "repos/$Repo/git/matching-refs/tags", '--jq', '.[] | "\(.ref|sub("refs/tags/";"")) \(.object.type) \(.object.sha)"')).Output
foreach ($line in ($refs -split "`n" | Where-Object { $_ })) {
    $parts = $line -split ' '
    $name, $type, $sha = $parts[0], $parts[1], $parts[2]
    if ($type -eq 'tag') {
        $kind = 'annotated'
        # The ref points at the TAG OBJECT; dereference to the commit, which is
        # what anyone checking these actually wants to compare against.
        $deref = Invoke-Gh @('api', "repos/$Repo/git/tags/$sha", '--jq', '.object.sha') -AllowFailure
        if ($deref.ExitCode -eq 0) { $sha = $deref.Output }
    }
    else { $kind = 'LIGHTWEIGHT' }
    Write-Host ("  {0,-10} {1,-12} -> {2}" -f $name, $kind, $sha.Substring(0, 7))
}
Write-Host ""
Write-Host "'annotated' is what docs/versioning.md rule 3 requires." -ForegroundColor DarkGray
