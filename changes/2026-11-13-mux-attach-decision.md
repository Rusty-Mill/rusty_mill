---
category: Changed
changelog: rusty_mux shared mode lets guests attach only to blocks they own or were granted; recorded as a design decision.
---
## 2026-11-13 - rusty_mux shared mode attach rule (D238)

- **Changed:** the rusty_term pack records D238: in shared mode (D232) guests list everything but attach only to blocks their principal owns or was granted (C77); the server account's shell is not attachable by guests, and the server account attaches anywhere. Docs only; implementation is PR #1662.
- **Known limitations:** relayed by the coordinator, not yet confirmed by Nano on PR #1662.
