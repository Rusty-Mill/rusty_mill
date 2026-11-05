---
category: Changed
changelog: rusty_mux: record Nano's acceptance of the C46 orphan rule for agent blocks (design only).
---
## 2026-11-05 - C46 orphan rule accepted

- **Changed:** records Nano's answer to question A of `docs/projects/mux/AGENT-HOSTING.md`: a block whose owner is gone past the 30 s reconnect grace is orphaned (it stays, input is closed, only the owner's principal may close it). Pack row D85 and a mux DECISIONS row.
- **Known limitations:** design only. Questions B and C, and the rest of each row's shape, stay Proposed.
