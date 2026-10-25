---
category: Fixed
changelog: Regenerate docs/WORKSPACE-MAP.md, stale on main after several PRs merged outside the queue.
---
## 2026-10-25 - Regenerate the workspace map

- **Fixed:** `docs/WORKSPACE-MAP.md` no longer matched the workspace after #1010–#1014 merged directly while CI could not run, so `dependency policy` failed on every PR and queue entry. Regenerated with the CI command (all-features metadata); `--verify` passes.
- **Known limitations:** none.
