---
category: Changed
changelog: rusty_mux: record Nano's answers to agent-hosting questions B (descriptor-passing public API) and C (server stop only hangs up owned blocks) (design only).
---
## 2026-11-05 - Agent-hosting questions B and C answered

- **Changed:** records Nano's answers to questions B and C of `docs/projects/mux/AGENT-HOSTING.md`. B: C75 may add descriptor-passing public API to `rusty_libc`, `rusty_tokio` and rustils `platform`/`platform-linux`. C: stopping the server only hangs up owned blocks, with no SIGKILL. Pack rows D86 and D87, plus two mux DECISIONS rows.
- **Known limitations:** design only. Every row's remaining shape and wire numbers stay Proposed.
