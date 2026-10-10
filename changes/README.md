# Change fragments

One file per PR. Two PRs never edit the same lines, so they never conflict
(the shared `RELEASE_NOTES.md` and `CHANGELOG.md` made every PR collide at the
top of the file).

## Add one with your PR

`changes/<YYYY-MM-DD>-<slug>.md`: the date you open the PR, a lowercase
hyphenated slug of the change. Not the PR number; it is not known until the PR
exists.

```
---
category: Fixed
changelog: One line for the changelog, in the style of the existing entries.
---
## 2026-10-10 - Title of the change

- **Fixed:** what changed and why, in the style of RELEASE_NOTES.md.
- **Known limitations:** stated plainly.
```

- `category` is one of Added, Changed, Deprecated, Removed, Fixed, Security.
- The body is the release-notes entry. It must start with
  `## <the file's date> - <title>`.
- Front matter has exactly the two keys above, one line each.

## Do not edit `RELEASE_NOTES.md` or `CHANGELOG.md`

They are archives up to 2026-10-10. This directory is the live log. Per-crate
logs under `crates/<name>/` are unchanged.

## Read it

```
python3 .github/scripts/changes.py check       # validate (CI runs this as a test)
python3 .github/scripts/changes.py notes       # release-notes view, newest first
python3 .github/scripts/changes.py changelog   # changelog view, grouped by category
```
