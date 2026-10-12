---
category: Fixed
changelog: `rb_rlbot_core` no longer stalls on a client that stops reading, caps clients, drops silent connections and refuses stop, state setting and ping from a connection that has not introduced itself; `rb_rl_game_bridge` skips and counts an undecodable game frame instead of ending the session.
---
## 2026-10-12 - rb_rlbot_core and rb_rl_game_bridge: hardening after review of #623

- **Fixed:** writes to clients were blocking with a 2 s timeout per client, so several clients that stopped reading froze every tick one after another. They now go through a per-client queue on the non-blocking socket; a client more than 1 MiB behind is dropped.
- **Fixed:** an `accept` failure (out of descriptors) ended the whole run; it now costs that connection and is reported once per streak.
- **Added:** at most 64 clients; a connection with no connection settings within 10 s is dropped.
- **Changed:** `StopCommand`, `DesiredGameState` and `PingRequest` from a connection that has not sent its connection settings now drop it (a runner sends settings with an empty agent id first). Previously any socket could end the match.
- **Fixed:** `rb_rl_game_bridge` ended the session on one frame that did not decode; it is skipped and counted (`skipped_frames`), and 32 in a row is an error.
- **Known limitations:** agent ids are still unauthenticated (the first client to claim one holds it); lockstep still mis-attributes a second input per packet; see the board cards for the rest of the review.
