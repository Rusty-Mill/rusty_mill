---
category: Added
changelog: rusty_mux clients can take over their own session after a reconnect, keeping their id, attachment and input lease.
---
## 2026-10-29 - rusty_mux: client takeover with resume tokens

- **Added:** `Features::TAKEOVER`. Each Welcome carries a takeover token. A Hello that presents a still-connected client's current token continues that client on the new connection, with the same id, name, attached block and input lease, and with a fresh screen of that block. The old connection is closed. Phase 3 step 2a; DECISIONS 2026-10-29.
- **Known limitations:** no client uses it yet; the SSH transport and heartbeat come in step 2b. Any peer the socket accepts can present a token, which is fine for the single-user socket but not for shared mode.
