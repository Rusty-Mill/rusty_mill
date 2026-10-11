---
category: Added
changelog: Scope note `docs/design/RLBOT-PORT.md` for porting the RLBot application (launcher, GUI, core, match config), with options, reuse, wire-protocol outline, crate layout and first slice.
---
## 2026-10-11 - RLBot application port: scope

- **Added:** `docs/design/RLBOT-PORT.md`. Reads RLBot core at `rc17` (launch arguments, bot process contract, TOML config parser, match lifecycle), the schema repository (59 tables, 11 structs, 50 enums, 6 unions) and the launcher and GUI descriptions. Finds that the game-facing bridge is closed source under an agreement with Psyonix, so a native core against the real game is blocked; compares a Rust manager over the real core (A), a sim-backed core with no game (B) and the blocked option (C); recommends B first; lists reuse (`rusty_flatbuffers`, `rb_rlbot_wire`, `rb_env`, `rusty_sandbox`, `rusty_serve`), missing pieces, crate layout, build order and a first slice that runs `rb_chaser_bot` against a sim-backed server.
- **Known limitations:** scope only. `Bridge.dll`, most of the server, the GUI and launcher sources and the botpack were not read; the schema was read from `main`, not core's pinned submodule; the schema repository has no licence file. The recommendation is the owner's decision.
