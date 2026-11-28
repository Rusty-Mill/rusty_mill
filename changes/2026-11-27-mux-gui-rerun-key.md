---
category: Added
changelog: The mux GUI re-runs a held block's ended command with Ctrl-] r (card #1634, D338).
---
## 2026-11-27 - mux GUI key for re-run

- **Added:** `Ctrl-] r` in the GUI sends `terminal.rerun` for the focused block under its lease, as the TUI does. A refusal (the program still runs, a restored or agent's block, another client holds the input) shows as a "Not re-run" notice with the host's reason and leaves the lease. `Source::reruns`/`rerun` are the GUI's seam. Decision gui-design D101.
- **Known limitations:** no palette row; an agent block's re-run through the orchestrator is not built.
