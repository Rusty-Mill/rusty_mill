---
category: Added
changelog: rusty_worktree gives mill-runner and BBP's confined harnesses git worktrees that share one object store; a confined harness can get a worktree checkout it alone may write.
---
## 2026-10-21 - Shared worktrees for confined harnesses (Stage 3a)

- **Added:** `rusty_worktree` (libs): worktrees of one repository through the git CLI, with `GIT_*` variables removed, hooks off, and the git directory always named explicitly. Taken out of mill-runner, which now uses it.
- **Changed:** mill-runner's worktree adds no longer run the repository's hooks.
- **Added:** a BBP launcher sandbox with `"checkout": true` gets a worktree of `--repo-path` at `HEAD` as its workspace, removed when the harness stops; the object store is readable, not writable. `HOME` and `TMPDIR` of every confined harness move to a scratch directory beside the workspace. Orch's pool `sandbox` accepts `checkout`.
- **Tests:** acceptance mode `shared-worktrees`: two confined tasks report the same git common directory and the checked-out file, write their own worktree, and are denied the other's worktree and the object store.
