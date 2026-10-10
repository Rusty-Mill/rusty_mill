---
category: Changed
changelog: **Release notes and changelog are now one fragment file per PR under `changes/`** (`changes/README.md`), so concurrent PRs stop colliding in `RELEASE_NOTES.md` and `CHANGELOG.md`, which become archives up to 2026-10-10. `.github/scripts/changes.py` validates fragments (run as a test in CI) and renders the notes and changelog views.
---
## 2026-10-10 - Release notes: one fragment file per PR

- **Changed:** every PR used to insert at the top of `RELEASE_NOTES.md` and `CHANGELOG.md`, so any two PRs in flight conflicted and each merge forced the others to be re-integrated and re-run through CI (about 35 minutes for the Windows shards). A PR now adds `changes/<YYYY-MM-DD>-<slug>.md` holding its release-notes entry and a one-line changelog entry; no shared file is edited. `RELEASE_NOTES.md` and `CHANGELOG.md` stay as archives up to 2026-10-10 and say so at the top.
- **Added:** `.github/scripts/changes.py` (`check`, `notes`, `changelog`) and its tests; CI's existing plan-script test job validates every committed fragment, so a malformed one fails the PR.
- **Why not append-only:** appending at the end of the same file is still one insertion point, so two PRs conflict there too. Separate files are the only layout where concurrent PRs touch disjoint paths.
- **Known limitations:** nothing stops a PR from editing the two archive files the old way; the notice at the top of each and `changes/README.md` are the only guard. Fragments are not folded back into the archives (nothing consumes the root files at release time); add a fold step if that changes.
