---
category: Added
changelog: rusty-mux-ctl terminal.rerun starts a held or restored block's command again, in place, under the lease.
---
## 2026-11-24 - One-key re-run of a held block (C45, card #1569)

- **Added:** `terminal.rerun` block call. A block whose program is gone and was started by this server run (held on exit) starts its stored command again, in the same block, under the input lease; the screen stays and a `re-run at` line says when. Restore still never runs anything by itself (D72). Needs the existing `Features::HOLD`; no new feature bit or store field.
- **Known limitations:** a block restored after a restart is refused (its launcher is not recorded); an agent's block is refused (it re-runs through the orchestrator, not yet built); the environment of the first run is not kept, so the re-run gets the server's. No GUI key yet: use `rusty-mux-ctl terminal.rerun`.
