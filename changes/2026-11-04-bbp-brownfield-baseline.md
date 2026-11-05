---
category: Added
changelog: `bbp open --base` runs the build suite on the base first and refuses a red one; candidates may not change base tests on a reversible claim (#598).
---
## 2026-11-04 - BBP brownfield baseline (#598)

- **Added:** `bbp open --mode claims --base REF --repo-path PATH --work DIR` resolves REF to a full commit and runs the task's frozen build suite on a clean checkout of it, sandboxed, before anything is written. A red base refuses the open `baseline_red`, naming each red profile and the one profile-set edit that lets it open. A green base becomes the profile set's `baseline`, and its report (`<dir>/baseline/<sha256(task)>.json`, with the base's test files) is frozen as `task_opened.baseline_digest`.
- **Added:** `base_test_changed`. On a task with a baseline, a candidate whose diffs modify or delete a test file that exists at the base is refused unless its claim is `reversible: false`, which the core never accepts without Nano. Added test files are fine.
- **Security:** with a holdout suite (`--holdout-profiles`/`--golden`), the baseline runs only once the suite's private paths are known not to overlap the build's read roots, the repository or the work root, symlinks resolved. Otherwise the open is refused `bad_claims_config` before the base is checked out or anything runs.
- **Fixed:** opening a task holds a per-task lock (`<dir>/open/<sha256(task)>.lock`) from the already-opened check through the append, and stages the holdout suite, baseline report and profile set only after that check. A duplicate or losing concurrent open writes nothing, so it can no longer replace the files a committed `task_opened` froze.
- **Fixed:** a pinned `baseline` is enforced on every candidate, not only on tasks with protected paths, and must be a full commit id. Builder workspaces start at the baseline instead of `HEAD`.
- **Known limitations:** the red detail names profiles, not individual tests, because reports carry profile results only. The Survey lens that seeds behaviour claims ships with the lens kinds in a later change.
