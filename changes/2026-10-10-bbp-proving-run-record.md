---
category: Changed
changelog: **`rusty_bbp_host`: first live proving run recorded (`docs/proving/2026-10-09-slug-1.md`).** Escalated on the iteration budget after 5 candidates, every runner run failing to exec `rustc` in the sandbox on WSL2; the Reviewer never ran. Record and runbook defects only; no code change.
---
## 2026-10-10 - rusty_bbp_host: first proving run record

- **Added (docs):** `crates/apps/rusty_bbp_host/docs/proving/2026-10-09-slug-1.md`, the record of the first live run of the Blackboard Protocol under `bbp mod` (Claude Code as Planner, Coder, Tester; Reviewer never reached). Outcome: `escalated`, `Budget(Iterations)`, 5 candidates, 5 failed runs.
- **Known limitations:** not a pass and not an answer to Q3. Ran in WSL2 (the runner is Unix-only) after two aborted attempts (expired harness login; a symlinked `~/.cargo/config.toml` outside the read roots). Every sandboxed run failed with `rustc` "Permission denied" and the cause is not established; the record lists six runbook/prompt problems and notes that `LSP` appears in `system/init` for roles launched with `--tools ""`. The machine-specific path edits to `profiles.json` are described in the record and not committed.
- **Corrected (record):** distinguishes the unused `request_decision` route from the observed prompt/discovery failure, identifies the per-principal `v6` operation conflict, and qualifies the positive code-assessment claim. Quoted evidence is unchanged; raw logs, local tests and the Landlock experiment remain author-reported.
