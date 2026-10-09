# Claude Code prompt: does a tape run need the game window focused, and can the runs be automated

Paste everything below the line into a new Claude Code session on the Windows
game machine, in a checkout of this repository (fetch `main` first).

---

You are working in `baileyrd/rusty_bullet` (a Rust port of Rocket League physics, scored against BakkesMod recordings). Read `AGENTS.md`, `tools/rb_tape_bot/README.md` ("Running" and "Facts settled against the running game"), `docs/adr/0059-rlbot-v5-and-bakkesmod-capture-setup.md`, `docs/research/BOT-RUN-SHEET.md` (session 1) and `bakkesmod-plugin/rusty_bullet_capture/README.md` first.

## Why

ADR-0059 and the run sheet say Rocket League pauses freeplay when its window loses focus, so the operator must keep the game focused for each tape. The owner reports the opposite: the tapes were running before the game was even in focus. That claim is unverified either way (the run sheet's own evidence is one freeze that may have been the Escape pause menu). If focus is not needed, the eleven scenarios can be run by a script instead of by hand.

## Ground rules

- Offline only (freeplay). Never try to get around Easy Anti-Cheat.
- Do not install or change anything outside this repository without telling the owner first. RLBot v5, BakkesMod and the plugin are already installed from session 1; the verified run sequence is in the README.
- Captures go to `replays\` (gitignored personal data); never commit them. No credentials in files or logs.
- Match the repo standards: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --color never -- -D warnings`, `cargo test --workspace`; for `tools/rb_tape_bot` (its own `[workspace]`) the same clippy and `cargo test`.
- Stop at the first stage that fails and report exact output; do not improvise past it.

## Stage 1: does focus matter? (do this first, change no code)

Run `prompt_dodge` (the control scenario) four ways and capture each (`replays\focus_<name>.jsonl`):

1. **focused**: the game window in front and clicked into, as in session 1. The baseline.
2. **behind**: start the capture and the match, then bring another full-screen-ish window (a terminal or browser) in front of the game without minimising it, leave it for the whole tape.
3. **minimised**: minimise the game window after the match starts, leave it for the whole tape.
4. **other monitor / alt-tabbed**: if there is a second monitor, focus a window there; otherwise skip and say so.

Run `target\release\rb_probe.exe [seconds]` alongside each (the passive client in `tools/rb_tape_bot`) so core's packets and frames per second are logged independent of the plugin. For every run record:

- frames in the capture, capture duration, mean frames per second, and the largest gap between consecutive timestamps (a paused game shows no frames or a multi-second gap);
- `rb_probe`'s once-a-second lines: match phase, frame number, packets per second;
- `rb-verify --scenario tools\rb_tape_bot\scenarios\prompt_dodge.json --against replays\focus_<name>.jsonl`: lag, position error (mean/max), and whether the dodge fired (the car's forward speed at the end, about 500 uu/s in the port's prediction);
- whether the tape began while the window was unfocused, and whether the car's start state was set at that time.

Judge: **focus is not needed** if runs 2 and 3 match run 1 (about 120 frames per second, no gap over 0.1 s, lag 0 to 2, error within the 2.8 uu mean run-to-run noise floor from session 1). **Focus is needed** if a run pauses, gaps or stalls the tape; then say which of behind or minimised does it. Write the result into `docs/research/BOT-RUN-SHEET.md` as "Session 2, stage 1" with a table (run, frames, fps, largest gap, lag, mean/max error, dodge fired) and correct the focus claim in `tools/rb_tape_bot/README.md` and ADR-0059 to what you measured (keep the ADR's other content; append a dated "Revisited" note rather than rewriting history).

If focus is needed, stop after writing it up and report; automation then needs a window-focus step and the owner decides whether it is worth it.

## Stage 2: automate the runs (only if stage 1 says focus is not needed)

Goal: one command runs every scenario in `tools\rb_tape_bot\scenarios\` unattended, capturing and scoring each.

1. **Capture start and stop without the console.** BakkesMod console commands cannot be driven from outside the game. Extend `bakkesmod-plugin/rusty_bullet_capture` with a trigger: it polls (at most once a second) for a job file at a fixed path under the BakkesMod data folder, and `{"start": "<absolute path>.jsonl"}` begins a capture and `{"stop": true}` ends it, exactly as `rb_capture_start` and `rb_capture_stop` do today (reuse their code; do not duplicate the writer). Keep the console commands working. Document the file format in the plugin README. Build it as the README describes and place the DLL; tell the owner before overwriting the installed one and keep a copy of the old DLL.
2. **Match start without the GUI.** ADR-0059 notes core's socket accepts `MatchConfiguration` from a small program (the `rlbot` crate's `start_match` example). Write a runner as a second binary in `tools/rb_tape_bot` (for example `src/bin/rb_run_tapes.rs`) that, for each scenario: ensures core is running (start `RLBotServer.exe` if not), writes the capture-start job file, sends a Freeplay match with state setting enabled and the scenario's bot (the per-scenario `bots/<name>.bot.toml` already sets `RB_TAPE`), waits for the tape length (the bot prints it; use the scenario's `total_ticks` plus a margin from `rb_scenario`), writes the stop job file, stops the match. Keep it small: no new dependency beyond what the crate already uses, `Result` and `?` throughout, no `unwrap` outside tests.
3. **Scoring.** After the runs, a short script (PowerShell or a `rb-verify` call per scenario) writes `replays\batch_<timestamp>\results.md` with, per scenario: lag, position error mean/max, first tick over 10 and 100 uu, frame count and largest gap. A hole of about 5 ticks per capture is known (O009); report it, do not hide it.
4. **Run it twice** over all eleven scenarios so each has two captures and a run-to-run noise floor, and report which scenarios were not repeatable.

Acceptance: a single command from the repository root runs all eleven scenarios with nobody touching the machine, `results.md` lists eleven rows, and the first frame of each capture is within 5 uu of its scenario's start location.

## Stage 3: write it down

- Update `tools/rb_tape_bot/README.md` ("Running": the one-command path first, the manual GUI path kept as the fallback), `docs/research/BOT-RUN-SHEET.md` (session 2 results), `docs/research/RESEARCH-BACKLOG.md` (any new open items with their evidence) and `CHANGELOG.md`.
- One ADR (next free number, template `docs/adr/TEMPLATE.md`) for the automation: the job-file trigger, the runner, and the focus finding.
- Run the repository gates listed above. Commit on a new branch (not `main`) with the attribution lines from your system reminder. Do not push and do not open a PR; report the branch name.

## Report format

Lead with "Next action: pick a/b/c below (about 1 min)". Then at most five numbered steps: the stage reached, the focus verdict with its numbers, what ran, anything that failed with exact output. End with options a/b/c. State plainly what was not verified.
