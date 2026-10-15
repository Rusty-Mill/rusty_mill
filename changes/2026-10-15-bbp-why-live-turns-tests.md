---
category: Added
changelog: **Tests pin what the live-turn count decides for `bbp why` and shadow mode.** A turn-based BBP task is replayed at one and at two live turns through the real engine. Tests only.
---
## 2026-10-15 - BBP host: `bbp why` over one and two live turns (tests only)

- **Added (tests):** `rusty_bbp_host/tests/why_live_turns.rs` runs one script on a turn-based task at one live turn and at two, through the real engine, and replays it with `bbp why`. At one: every grant carries its shadow comparison (agree, fallback with observers in the set, disagree with the addressed role) and the record `bbp shadow` kept matches the replay (same rules, same wake set). At two: no shadow record, no wake set, no comparison, and an observer-only board is still the default holder. An Explicit grant (a yield) is labelled while another turn is live, and carries no set or comparison.
- **Unchanged:** the engine, the host, every log byte. No `open` bypass: an observer pick above one live turn is not reachable through the engine (turn-based ignores `wake_set`; `open` refuses event-driven above one live turn), so it stays a unit-level fact in `tests/wake.rs`.
- **Known limitations:** none new.
