---
category: Fixed
changelog: The bbp build sha and the bbp pairs pin no longer report clean for changes hidden by git config, index bits or inherited git environment.
---
## 2026-11-02 - bbp clean-tree check sees changes git settings hide

- **Fixed:** `build.rs` (`BBP_BUILD_SHA`) and the `bbp pairs` pin could report a changed tree as clean (#1361). Both now share one check, `src/clean_tree.rs`, which reports dirty for: untracked files listed by `core.excludesFile` or `.git/info/exclude` (only `.gitignore` files are read); mode changes under `core.fileMode=false` (Unix); submodule changes under `diff.ignoreSubmodules` or `submodule.<name>.ignore`; and edits run from the crate directory to files elsewhere in the checkout. Path lists are read NUL-separated and untrimmed, so a file named by whitespace alone still counts. Inherited `GIT_DIR`, `GIT_WORK_TREE`, `GIT_INDEX_FILE` and `GIT_COMMON_DIR` (from a git hook running cargo) are removed.
- **Changed:** a tree with any assume-unchanged or skip-worktree entry (set by `git update-index`, `core.ignoreStat` or sparse checkout) is dirty even if the file is unedited, since git cannot be made to look past those bits.
- **Changed:** `.claude/settings.local.json` is now in `.gitignore`. A personal file ignored only by a global excludes file now makes a build `-dirty`; ignore it in `.gitignore` or remove it.
- **Known limitations:** `core.fsmonitor` and `core.untrackedCache` can still serve stale answers; neither is overridden.
