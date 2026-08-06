# scripts

Release tooling. Two PowerShell scripts, both driving the `gh` CLI against the
GitHub API — neither needs a local clone, though `New-RustyTlsReleases.ps1`
reads `CHANGELOG.md` and defaults to finding it in the current directory.

| Script | Does |
| --- | --- |
| `Cut-RustyTlsTags.ps1` | Creates the annotated version tags |
| `New-RustyTlsReleases.ps1` | Creates the GitHub releases for those tags |

**Order matters: tags first, releases second.** `New-RustyTlsReleases.ps1`
never creates a tag. It refuses to create a release for a tag that does not
already exist, because a release is a label on a commit and inventing the
commit is not its job.

## Requirements

`gh`, authenticated with a token carrying `repo` scope (contents: write).
Check with `gh auth status`.

PowerShell 5.1 or later. Both run on `pwsh` for Linux and macOS as well as
Windows PowerShell; nothing in them is platform-specific.

## Running them

Both support `-WhatIf`. Use it — each does more verification than it does
work, and `-WhatIf` runs all of the former and none of the latter.

```powershell
./Cut-RustyTlsTags.ps1 -WhatIf                  # what would be tagged
./Cut-RustyTlsTags.ps1                          # tag, prompting per tag

./New-RustyTlsReleases.ps1 -WhatIf              # what would be released
./New-RustyTlsReleases.ps1                      # create and publish in one go

./New-RustyTlsReleases.ps1 -Draft               # create as drafts, to read first
./New-RustyTlsReleases.ps1 -PublishDrafts       # ...then release what you drafted
```

`-Draft` and `-PublishDrafts` are the two halves of the same workflow, and
passing both is an error rather than a no-op. Drafting first is worth it the
first time: release bodies come out of `CHANGELOG.md` and how they render is
easier to judge in the UI than in a diff.

**A draft is not attached to its tag.** Its URL reads
`releases/tag/untagged-<hash>` until it is published, at which point it becomes
`releases/tag/vX.Y.Z`. `-PublishDrafts` checks that transition on every release
rather than trusting the API's exit code, because "succeeded, and here is a
draft URL" is a state the API can return.

`-PublishDrafts` publishes in ascending version order, so `Latest` lands on the
newest rather than on whichever happened to go last. It only touches drafts
whose tag is listed in `$Releases`, and it creates nothing — a tag with no
release at all is reported so a normal run can create it.

Run `New-RustyTlsReleases.ps1` from the repository root so it finds
`CHANGELOG.md`, or pass `-ChangelogPath`.

## Why these are longer than four lines each

Both are mostly verification, and each check exists because of something that
actually went wrong.

**`gh release create` is not used, anywhere.** It creates a *lightweight* tag
when the tag does not already exist, and `docs/versioning.md` rule 3 requires
annotated tags. `Cut-RustyTlsTags.ps1` uses the Git Data API in two steps
(`POST git/tags`, then `POST git/refs`), which produces a real annotated tag
object.

**Every pin is verified, not trusted.** Before tagging a commit, the script
fetches `Cargo.toml` *at that exact commit* and requires it to declare the
version being tagged. It also checks the commit is a merge commit
(`docs/versioning.md` rule 2) and an ancestor of the default branch. A pin
that fails is reported and skipped rather than tagged wrongly.

**Neither script may pass vacuously.** `New-RustyTlsReleases.ps1` refuses to
publish a release whose changelog section is missing or empty, rather than
creating one with an empty body. This is the same concern as the
`changelog-parity` and zero-tests guards in `.github/workflows/ci.yml`: a
check that silently finds nothing to check is worse than no check, because it
reports success.

**Tables render through `Out-String -Width`, never `Format-Table -AutoSize`
alone.** `-AutoSize` emits *nothing* when the host reports no console width
(`BufferSize.Width = -1`), which is every redirected, piped or CI run — so the
verification table would vanish in exactly the runs where someone captured the
output for the record.

## Current state

Every version on `main` is tagged, and `$Tags` in `Cut-RustyTlsTags.ps1` lists
all of them. A run today reports `skip (exists)` for every entry and creates
nothing — the expected steady state. Verification still runs, so a no-op run
is a cheap re-check that no published tag has drifted from the commit whose
`Cargo.toml` declares its version.

For the next release, add one row to `$Tags`, add one entry to `$Releases` in
`New-RustyTlsReleases.ps1`, and run them in that order.

## Two things not to "fix"

**There is no `v0.8.0`, and there should not be.** rusty_tls#57 carried two
version bumps in one merge, so `main` went `0.7.0` straight to `0.9.0` and no
commit ever declared `0.8.0`. rusty_tls#64 folded the orphaned `0.8.0` entries
in `CHANGELOG.md` and `RELEASE_NOTES.md` into `0.9.0`, the release the work
actually shipped in. The gap in the tag sequence is the accurate record.

**`v0.1.0` warns that it is not a merge commit, and that warning is correct.**
PR #3 was squash-merged, so the commit has one parent. Its `Cargo.toml` still
says `0.1.0` and it is the first commit on `main` to declare a version at all,
so the tag is right and the warning is the script correctly reporting the one
exception rather than hiding it.
