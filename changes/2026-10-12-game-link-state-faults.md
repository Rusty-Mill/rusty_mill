---
category: Fixed
changelog: `rb_rl_game_bridge` no longer silently drops a game event that carries state it cannot get back (spawns, destroys, the match phase, scores); such an event that cannot be read is a `WireError::State` fault that ends the session with the event named.
---
## 2026-10-12 - rb_rl_game_bridge: unreadable state events are faults, not skips

- **Fixed:** the tolerant decoder turned any event with a missing field, an unknown enum or no body into `GameEvent::Other`, including `PlayerSpawn`, `GameSpawn`, `BallSpawn`, `GoalSpawn`, `PickupSpawn`, `ActorDestroyed`, the match phase and scores. The game announces those once and the known commands include no resend or snapshot, so a lost `PlayerSpawn` left a spawn acknowledgement that never came and a lost phase change left the phase wrong for the rest of the match. They are now `WireError::State { tag, cause }`; `carries_state(tag)` says which.
- **Unchanged:** physics, boost, the clock, pad state, hits and stats still decode to `Other` when unreadable (they are re-sent or untracked), and the strict analysis decoder is as before.
- **Known limitations:** there is no resync inside a map: the protocol's only full announcement is a new map's `GameSpawn`. The session still ends on the fault; continuing while "desynced" until the next map was considered and left out, since bots would get nothing meanwhile.
