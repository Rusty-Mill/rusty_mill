---
category: Added
changelog: The rusty_conhost console-host design (#1222) and its spike evidence are in docs/projects/term/conhost/.
---
## 2026-10-29 - rusty_conhost design in the repository

- **Added:** `docs/projects/term/conhost/design.md`, the design for our own
  Windows console host (#1222), with the owner's decisions recorded and the
  exact dependency, admission and public API changes the reviewer asked for
  (§11), and the engine-thread concurrency, shutdown and memory model (§7).
  The recon, spike and report notes and both review rounds sit
  alongside it. Documentation only; no code changes.
