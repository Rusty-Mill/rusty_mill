---
category: Changed
changelog: `rb_tape_bot` uses `rb_rlbot_client` / `rb_rlbot_wire` instead of the external `rlbot` crate and joins the workspace (standalone `Cargo.lock` retired); `rlbot` and `rlbot_flat` leave the dependency graph (RLBot port stage 4).
---
## 2026-10-10 - rb_tape_bot on the in-repo RLBot client; `rlbot` removed (RLBot port stage 4)

- **Changed:** `rb_tape_bot` (`crates/apps/rocket_league/rusty_bullet/tools/rb_tape_bot`: `rb_tape_bot`, `rb_tape_hive`, `rb_probe`, `rb_match_log`, `rb_run_tapes`) now uses `rb_rlbot_client` and `rb_rlbot_wire` instead of the external `rlbot` 0.6.0 crate. Same behaviour by construction: the frame gate, start state and tape indexing are unchanged; the non-blocking poll loops became `recv_timeout(2 ms)`.
- **Changed:** the package is a workspace member (it was in `exclude` with its own `[workspace]`). Its standalone `Cargo.lock` (which carried `rlbot`, `rlbot_flat`, `planus`, `mio` and `kanal`) is deleted; the root `Cargo.lock` only gains the package itself. It now builds, tests and lints (`clippy -D warnings`) in CI. One clippy finding latent in `rb_match_log` is fixed (`is_multiple_of`).
- **Added:** `tests/processes.rs` runs the real bot and hive processes against a stand-in core: handshake, start state on the first physics-advancing packet, one input per frame (not per packet), car index other than 0 stays neutral, hivemind drives both cars from one clock. Plus unit tests for the controller mapping and start state, which the process tests alone did not pin.
- **Added:** `.cargo/config.toml` in the package keeps its build output in `tools/rb_tape_bot/target`, where `bot.toml`, `bots/*.bot.toml` and `rb_run_tapes` start the bots. A build from the repository root goes to the workspace `target/`, which core does not look in; the README says so.
- **Verified:** run once against the real game (RLBot v5.0.0-rc17): one-car pogo tape, match log and probe, and hivemind and two-team tape runs passed; the report is in the PR.
- **Known limitations:** the match-log tie-up and overtime paths were not exercised (the runs used an unlimited match, which has no regulation end; the commands now pass `--length five`); the scoring check (E) was not run. The last client changes (whitespace-only agent id guard, `Transport` refactor of the read loop) are covered by unit tests but were not re-run on the game.
