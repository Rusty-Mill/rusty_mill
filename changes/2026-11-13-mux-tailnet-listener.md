---
category: Added
changelog: rusty-mux-server --shared can listen on a tailnet address and serve the tailnet peers the allowlist names (C27 slice 2).
---
## 2026-11-13 - rusty_mux tailnet listener, slice 2

- **Added:** `rusty-mux-server --shared ALLOW --tailnet ADDR:PORT` (Linux, card #276). The address must be a tailnet one (`100.64.0.0/10` or `fd7a:115c:a1e0::/48`); loopback and `0.0.0.0` are refused. Each peer is looked up with `tailscaled` before it can say anything, and is served as a shared-mode guest only if the allowlist names it with a `ts:LOGIN` or `ts-node:NAME` line. A reload closes the connections it no longer names.
- **Added:** `rusty_mux_server::Tailnet` and `Sharing::tailnet` (library API), and `tailnet::is_tailnet_addr`.
- **Known limitations:** tailnet guests have no UDP and pass no descriptors; transport shows as `socket`. No wire change, no new dependency.
