---
category: Added
changelog: rusty_orch run --cancel stops a hosted goal, waits until its processes are reaped, and retires its assignment.
---
## 2026-10-23 - rusty_orch: cancel a hosted goal

- **Added:** `rusty_orch run <goal> --state <dir> --bbp <cfg> --cancel <reason>`. It takes the goal's assignment over at a new generation, so any earlier holder is fenced out, then cancels each hosted BBP task through the new `TaskHost::cancel`. The host returns only once the moderator has released its lock, which BBP does after reaping every harness and runner, so the task's slot is free when the command returns. The card's execution is then recorded as cancelled and the assignment retired; a later `run` on that state is refused. Re-running the cancel completes an interrupted one and changes nothing after a complete one. New acceptance mode `orch-cancel`.
- **Added:** BBP configuration `end_grace_secs`, passed to `bbp mod --end-grace-secs`, bounds how long a harness may keep running after its turn ends.
- **Known limitations:** Millboard is told the new generation, not the cancelled state: it has no operation for an execution's terminal state yet. A plain `run` after a cancel interrupted before retirement reports the BBP task's cancellation as a failure rather than finishing the cancel. Path leases (3c) and fair admission (3d) are separate slices.
