---
category: Added
changelog: rusty-mux can attach to a server on another machine with `--ssh HOST`, through one OpenSSH socket forward.
---
## 2026-10-29 - rusty_mux: `rusty-mux --ssh HOST`

- **Added:** `rusty-mux --ssh HOST <socket> [block]` forwards a private local socket to `<socket>` on HOST with one `ssh -N -L` and attaches through it. The SSH login is the identity. The forward stops and its directory is removed when the client exits. The forward never joins or leaves behind a shared SSH master, whatever the user's config says, and the socket paths (absolute) reach ssh exactly as given. Needs OpenSSH 8.7 or later. `RUSTY_MUX_SSH` names another `ssh` program. Phase 3 step 2b; DECISIONS 2026-10-29.
- **Known limitations:** Unix only for now. No reconnect yet: if the forward drops, the client exits, as it does when a local server goes away. Servers that refuse stream-local forwarding need the `rusty-mux-ctl bridge` fallback, which comes next.
