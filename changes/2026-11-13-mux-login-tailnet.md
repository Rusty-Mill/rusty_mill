---
category: Added
changelog: rusty-mux-server --login MAP --tailnet ADDR:PORT admits tailnet peers named in the account map.
---
## 2026-11-13 - rusty_mux login mode takes tailnet peers (C27 slice 3)

- **Added:** `rusty-mux-server --login MAP --tailnet ADDR:PORT` (Linux). The map takes `ts:LOGIN` and `ts-node:NAME` rules to a named account; `=` is refused and `*` never matches a tailnet peer. Peers are named by the local `tailscaled`, and the relay preface carries the principal unchanged, so there is no wire change (card #276, rusty_term D263).
