---
category: Added
changelog: The mux TUI re-runs a held block's ended command with Ctrl-] r (card #1569, D338).
---
## 2026-11-26 - mux TUI key for re-run

- **Added:** `Ctrl-] r` in the TUI sends `terminal.rerun` for the focused block under its lease. A refusal (the program still runs, a restored or agent's block) rings the bell and leaves the lease. Block connections now ask for `Features::HOLD`, which the server already grants.
- **Known limitations:** no GUI key yet; an agent block's re-run through the orchestrator is not built.
