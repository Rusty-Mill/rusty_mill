---
category: Added
changelog: **`rusty_mill_board`** serves several applications of a monorepo from one board with enforced projects, and `mill-runner` can be limited to one project's paths.
---
## 2026-10-14 - Mill Board: projects

- **Added:** cards belong to a `project`; `--projects FILE` names the projects and the repository paths each may change (`default` owns the rest). `board_projects`, `card_assign` (Triage; drops the claim), a `project` argument on `card_add`, `card_take` and `board_list`, and a project switcher in the UI. A credential in `--identities` can carry `"projects": [...]`: it cannot see, take or change other projects' cards. The check is inside every write's compare-and-swap, so a card re-assigned mid-call is refused. Cards stored before this read as `default`; the record layout is unchanged.
- **Runner:** `mill-runner --project NAME` (required on a multi-project board) only takes that project's cards and, after a stage succeeds, discards the whole run and fails the stage if it changed a path outside the project's paths or `--shared-path` (default `changes`, `Cargo.lock`). The sandbox write scope is still the worktree; paths are enforced at commit.
- **Tests:** `tests/projects.rs` (scope on every write, `take`, `assign`, a re-assignment race), the operation table with a limited credential, runner scope and discard against a scripted workspace, `git status` parsing and discard against real git, `tests/ui/projects.mjs` in headless Chromium. Every enforcement check was removed in turn and a test failed.
- **Known limitations:** open and shared-token boards still trust claimed actors. `board_events` shows other projects' event metadata (no card text). GitHub sync is project-blind. ADR-0003.
