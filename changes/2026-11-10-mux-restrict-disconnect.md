---
category: Security
changelog: rusty_mux restricts `disconnect-client` on shared servers to the server's own account, the target's own principal, or the creator of the target's session.
---
## 2026-11-10 - Restrict C33 force-disconnect (arc 1, card 507)

- **Security:** on a server several principals share, `disconnect-client` now succeeds only for the server's own account, for a client of the target's own principal, or for the principal that created the session the target is attached to. Before, anyone on the socket could disconnect anyone. A refusal is a plain `Failed` naming the rule (ctl exit 1); an unknown client is still exit 3. C46's owner refusal and the self-disconnect refusal are unchanged and come first.
- **Added:** `rusty_mux_core::Session::creator`, `Mux::set_creator` and `Mux::session_of_client`. The server records the creating client's principal on `create-session`.
- **Known limitations:** sessions restored from the store have no creator and belong to the server's account, so only it may disconnect their clients until they are re-created. Personal servers, where every client is the server's account, behave as before. Q36 (self-disconnect) stays open; it stays refused.
