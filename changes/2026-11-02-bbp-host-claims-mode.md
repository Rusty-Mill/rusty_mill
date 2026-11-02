---
category: Added
changelog: **`rusty_bbp_host`: claims mode.** `bbp open --mode claims` opens a claims task, and only on a host that can sandbox. Sessions get credentialed claims tools. The runner lists the test paths a candidate changes. `bbp inbox` lists open escalations, and `bbp human --dry-run yes` previews a decision.
---
## 2026-11-02 - rusty_bbp_host: claims mode (Let Em Loose #593, #595)

- **Added:** `bbp open --mode claims [--claims FILE]`.
  - It probes the sandbox and refuses `bad_claims_config(sandbox)` when the host cannot confine a run, or when given `--confine none`.
  - It refuses `bad_claims_config(test_paths)` when the profile set names no test paths.
  - Both refusals happen before anything is written (CL-44).
- **Added:** claims-mode MCP sessions (`BBP_SESSION=yes`).
  - Every call, views included, carries the session credential, which the host derives when the session starts and the model never sees.
  - The session keeps that credential, so a reassignment of the role (even back to the same principal) revokes a running session.
  - The tools are `post` for claims and evidence, `put_artifact` for diff, candidate and source, and `claims_delta`, `claim_card` and `claim_line`.
  - There is no `wait` and no per-turn binding.
- **Added:** `ProfileSet.test_paths`, a list of globs.
  - It is written only when non-empty, so stage-mode profile digests are unchanged.
  - The runner serves claims runs only sandboxed.
  - It fills `Report.tests_changed` with each matching file the candidate adds, modifies or deletes.
- **Added:** `bbp inbox --task T1,T2`, which lists escalated claims, overdue marks, held reruns and task totals as JSON. A closed or cancelled task lists nothing.
- **Added:** new human verbs: `claim`, `choose`, `extend-claim`, `withdraw-rerun`, `action-result` and `close`.
- **Added:** `--dry-run yes` on `bbp human`, which prints the events an action would commit.
- **Known limitations:** the moderator does not launch claims-mode runners or harnesses yet. Orch decides whom to run in claims mode, and that wiring is the next step. Flags take a value in this CLI, so the dry run is spelled `--dry-run yes`, and `bbp inbox` always prints JSON.
