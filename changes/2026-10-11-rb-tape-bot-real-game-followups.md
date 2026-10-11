---
category: Fixed
changelog: `rb_match_log --overtime-goal` now fires (it waited for overtime time left, which core reports as 0 or negative); `rb_rlbot_client` io errors name the failing socket call.
---
## 2026-10-11 - rb_tape_bot real-game follow-ups: overtime goal, named socket errors

- **Fixed:** `rb_match_log --overtime-goal` never shot. It required `game_time_remaining > 3.0` in overtime, but core reports 0.0 at the first overtime kickoff and then negative values (-39.0 at a goal 39 s in; RLBot rc17, found in the stage 4 real-game check). The decision is now `tie_up::overtime_goal_due(overtime, playing, sent)`, which does not look at the clock. The README's finite-match commands use `--seconds 480` and `720`, the windows that reached the tie-up and overtime in that check.
- **Changed:** `rb_rlbot_client` io errors name the socket call that failed (`read (poll)`, `set_nonblocking(false)`, `set_read_timeout(..)`, `write`). A timeout or interruption is passed through unchanged. A Windows run had died with a bare `Overlapped I/O operation is in progress (os error 997)`; the cause is not established, and this only makes the next occurrence say where it came from.
- **Known limitations:** the os error 997 drop did not reproduce in two later runs of about ten minutes each on this build. The overtime fix is not yet run on the game.
