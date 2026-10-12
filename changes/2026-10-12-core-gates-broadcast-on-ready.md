---
category: Fixed
changelog: `rb_rlbot_core` no longer broadcasts a self-paced backend to the bots while it is not ready, so after a game loads a new map the bots are not given a match with their own cars missing.
---
## 2026-10-12 - rb_rlbot_core: broadcast a self-paced backend only while it is ready

- **Fixed:** `Core` checked `MatchBackend::ready()` only before the match started. With a real game behind `LinkBackend`, a new map resets the folded state, `ready()` goes false until the bots' cars are back, yet the bots were still sent every packet the game produced, with the bots' own slots empty (default physics) and a human at a later index counted as enough players. Once started, a polled packet is now broadcast only if the backend is ready. Found in the review of #623 (board card b22).
- **Changed:** the contract of `MatchBackend::ready` is documented to include this. For `LinkBackend` that means a bot car that is destroyed mid-map (`ActorDestroyed`) stops the broadcast until the next map, since the bridge does not respawn it yet (board card b24); before, the bots kept receiving packets with a placeholder for the missing car.
- **Known limitations:** `should_distribute` still counts slots, not live cars; the gate is on readiness, which checks each configured bot's own car. Not run against a real Rocket League.
