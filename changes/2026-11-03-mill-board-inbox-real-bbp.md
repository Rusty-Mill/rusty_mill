---
category: Added
changelog: CI drives the Mill Board inbox against a real bbp host (#595).
---
## 2026-11-03 - Mill Board inbox against a real bbp host

- **Added:** `rusty_mill_board/tests/inbox_real_bbp.rs` runs the `BbpCli` adapter against the `bbp` named by `BBP_TEST_BIN`. It opens a sandboxed claims task, pauses it on a task total, and checks:
  - The inbox row reports the crossed total, its limit and one allowance.
  - Approve previews and commits exactly one allowance and resumes the task.
  - Resending the same decision is fenced out and adds nothing.
  - Refine on a task total answers, extends and resumes.
- **Added:** the confined acceptance lane builds the test and runs it as the `millboard-inbox` mode. The test is `#[ignore]` for ordinary runs. Once selected, it fails rather than skips when `BBP_TEST_BIN` is unset or the host misbehaves.
