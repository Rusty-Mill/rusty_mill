---
category: Changed
changelog: The rusty_mux roadmap marks Phase 2b (agent parity) done, accepted by the owner on 2026-10-28.
---
## 2026-10-28 - rusty_mux Phase 2b accepted

- **Changed:** ROADMAP marks Phase 2b done, accepted by the owner on 2026-10-28. The manual Windows check (#1212) failed the Ctrl-] prefix. After the fix (#1214), a retest showed Ctrl-] then `d` detaching and Ctrl-] then `|` splitting.
- **Known limitations:** not run on Windows: the other layout keys (`-`, arrows, `z`, `x`), reattach after a clean detach, and the two-account check (R10).
