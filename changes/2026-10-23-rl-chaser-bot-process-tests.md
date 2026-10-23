---
category: Added
changelog: rb_chaser_bot is tested as a built executable, against a stand-in core and against the simulated core (ball hit in a fixed time, same match twice).
---
## 2026-10-23 - rb_chaser_bot: process-level tests

- **Added:** `rb_rlbot_bridge/tests/chaser_bot.rs` runs the built `rb_chaser_bot` the way core runs it, through `RLBOT_SERVER_PORT` and `RLBOT_AGENT_ID`. Against a stand-in core it checks the agent id it announces, that it plays the car and team core gives it (car 1, orange, the same input the in-process chaser gives for team 1 and not the one for team 0), that a disconnect signal ends it with success, and that a core that closes mid-match or is not there at all ends it with failure. Against a lockstep `rb_rlbot_core`, a blue and an orange chaser process hit the ball by frame 3250 and two runs give the same packets up to the hit: the done-criteria of the RLBot port's first slice (`docs/design/RLBOT-PORT.md`).
- **Changed:** `rb_rlbot_bridge` takes `rb_rlbot_core` as a dev-dependency. Core depends on this crate's library, so this is a dev-only cycle, which Cargo allows for integration tests.
- **Known limitations:** the test starts the bots itself; core still has no launcher. The chasers take about 12.8 s of match time to reach the ball from the diagonal kickoff spots, so the bound is generous (20 s after the countdown).
