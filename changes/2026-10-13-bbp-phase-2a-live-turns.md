---
category: Added
changelog: **BBP several live turns (Phase 2a).** `max_live_turns` in `task_opened` allows up to that many concurrent turns, one per role, with per-turn deadlines and read counts, revocation of all on a transition, and message/byte slices reserved at grant; 1 keeps the previous behavior.
---
## 2026-10-13 - BBP several live turns (Phase 2a)

- **Added:** `rusty_bbp` accepts `max_live_turns` (1 to the roster size) at `open`. Above 1 the state's roles fill the free turn slots in `turn_order`, each with its own token, deadline and `reads_per_turn` count; any transition revokes every live turn; `turn_granted` carries a `slice` of `messages` and `bytes` taken from the unreserved headroom, so concurrent turns cannot jointly overshoot the task totals. New code `bad_live_turns`. `TaskState::turns` replaces `turn` (`turn()` returns the first live turn); `Card::turns` replaces `Card::turn`; `Spent::reads_this_turn` moved to `Turn::reads`.
- **Changed:** `max_live_turns = 1` is the default and behaves as before: all 31 traces and the model tests pass unchanged. The log schema stays 2: the new event fields are optional and old logs read as one live turn.
- **Known limitations:** `rusty_bbp_host` still runs one harness at a time and opens tasks with `max_live_turns = 1`; its moderator needs one harness per live turn before the cap can be raised there. No contention or throughput measurement yet. Concurrent turns go only to roles that serve the same state; consultation and human `ask` turns share the same cap.
