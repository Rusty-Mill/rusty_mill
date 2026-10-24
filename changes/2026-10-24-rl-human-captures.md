---
category: Added
changelog: Nine human-driven BakkesMod captures, gated in rb_verify_cli, plus ten raw clips, imported from the unmerged rusty_bullet server-clone branch.
---
## 2026-10-24 - Rocket League: human-driven captures from the server-clone branch

- **Added:** `rb_capture_ingest/fixtures/human/`, nine excerpts (2.4 MB) of the owner's September BakkesMod sessions with the recorded input on every tick: throttle and jumps, a boost run into the wall curve, side and diagonal dodges, an aerial hit, a wheels-down hit-tick jump, a wall climb over the ceiling fillet, a shot into the net. They are byte-identical copies from `baileyrd/rusty_bullet`'s `claude/rocket-league-server-clone-u74q45` branch (head `bcf8f69`), the one source branch ADR-0008 left unimported.
- **Added:** `rb_verify_cli/tests/human_captures.rs` seeds the port on each excerpt's first grounded, neutral frame, feeds it the recorded input and bounds car and ball error at the 2026-10-24 measurement plus a quarter, like `golden_captures.rs`. Main tracks eight of the nine within 2 uu car mean and `wall-climb-crest` at 17 uu, where the branch's own physics was at 169 and 364 uu on two of the same files (its README).
- **Added:** `human/raw/`, the ten whole clips (33 MB) the excerpts were cut from, for input-recovery and investigation work. No test reads them.
- **Not imported:** the branch's physics code and history. Main rebuilt the same features (wheel rays, suspension, flip torque, hitbox offset) independently since the branch forked on 2026-09-04 and is ahead on every excerpt above. The branch's findings, under its own FR-079 to FR-094 numbering, follow in a separate PR with an old-to-new map.
- **Known limitations:** the `hit-tick-jump` ball bound is today's error (189 uu mean, 1024 max after the hit), not parity. Steer, pitch, yaw and roll are recorded as zero throughout `hittickjump01`/`01b` (throttle is recorded), and `walldrive04` has a dodge with no `jump` press recorded.
