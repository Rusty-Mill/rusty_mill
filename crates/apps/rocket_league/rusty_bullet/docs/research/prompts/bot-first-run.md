# Claude Code prompt: install RLBot and do the first tape-bot capture run

Paste everything below the line into a new Claude Code session on the
Windows machine that has Rocket League and BakkesMod installed, opened in a
checkout of this repository (fetch `main` first).

---

You are working in the `baileyrd/rusty_bullet` repository (a Rust port of Rocket League's physics, scored against the owner's BakkesMod recordings). Read `AGENTS.md`, `docs/research/BOT-RUN-SHEET.md` (the plan you are executing), `tools/rb_tape_bot/README.md`, `docs/research/BOT-CAPTURE-PLAN.md` and `bakkesmod-plugin/rusty_bullet_capture/README.md` first.

## Goal

Get `tools/rb_tape_bot` (an RLBot v5 bot that has never run against the game) driving scripted scenarios while the BakkesMod plugin `rusty_bullet_capture` records them, then score each capture with `rb-verify --scenario <file> --against <capture>`. Execute the run sheet stage by stage and **stop at the first failed stage**, report what failed with the exact output, and do not improvise past it.

## Ground rules

- Offline only (freeplay or an offline/local match). BakkesMod and bots are blocked online by Easy Anti-Cheat; never try to get around that.
- Download software only from official sources (rlbot.org, the BakkesMod and RLBot GitHub organisations, rustup.rs, Microsoft). Show the owner each download URL and installer before running it; do not run anything that asks for admin rights without telling them first.
- Do not trust installation steps from memory: RLBot v5 is new and its docs move. Read the current docs at https://rlbot.org/v5/ (and the `RLBot/core` and `RLBot/rust-interface` repositories) before each step, and write down in your report any place where the docs disagreed with `tools/rb_tape_bot/README.md`.
- The owner's captures in `replays/` are gitignored personal data. Never commit them; commit only code, docs and numbers.
- No tokens or credentials in files or logs.
- Match the repo's standards: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --color never -- -D warnings`, `cargo test --workspace`. For the tape bot (own `[workspace]`): the same clippy and `cargo test` run inside `tools/rb_tape_bot`.

## Task 0: preflight

1. Report what is already installed: Rocket League (Steam or Epic), BakkesMod (version, and whether `rusty_bullet_capture.dll` is already in `%appdata%\bakkesmod\bakkesmod\plugins\`), Rust toolchain (`rustc --version`), MSVC build tools, CMake, Git.
2. Install what is missing: rustup (stable, MSVC target) if there is no Rust. Do not reinstall BakkesMod or the game.
3. In the repository: `cargo build --release -p rb_verify_cli` and confirm `target\release\rb-verify.exe --self` runs. Confirm `replays\` exists with the owner's captures (if it does not, say so; the first-run scoring still works from the new captures alone).

## Task 1: install and configure RLBot v5

1. Follow the official v5 install path (the RLBot GUI from https://rlbot.org/v5/). Install it and launch it once so it finds the game. If it asks where Rocket League is, use the game's real install directory and tell the owner what you chose.
2. Check how the GUI launches the game (Steam or Epic launch, any RLBot-specific launch flags) and whether BakkesMod still injects into a game started by RLBot. This is the biggest open risk: **BakkesMod must be injected into the same Rocket League process RLBot starts.** Find out the supported way to run both (for example starting the game through RLBot and injecting BakkesMod afterwards, or the reverse) and write the working sequence into `tools/rb_tape_bot/README.md` under "Running".
3. Build the bot: `cargo build --release` in `tools/rb_tape_bot`. Note the `rlbot` crate version in `Cargo.toml` (0.6.0) against what the installed RLBot core speaks; a protocol mismatch is a likely first failure. If they disagree, report both versions and propose the smallest fix (a crate bump is a dependency change: ask the owner before making it).
4. Register the bot: add `tools/rb_tape_bot/bot.toml` in the GUI (or the equivalent v5 mechanism), on one team, no opponents needed. Set `RB_TAPE` to a scenario path in the environment RLBot starts bots with (check how v5 passes the environment to a bot's `run_command`; if it does not, find the supported alternative and report it before changing the bot).
5. Set the match to offline, game mode freeplay or a one-player match with the ball present, no mutators.

## Task 2: BakkesMod plugin

1. If the plugin is not yet built, build it as documented in `bakkesmod-plugin/rusty_bullet_capture/README.md` against the owner's `BakkesModSDK` copy, and place the DLL in the BakkesMod plugins folder. If it is already built and loaded, just confirm `plugin load rusty_bullet_capture` works in the BakkesMod console (F6).
2. Check the console commands `rb_capture_start <file>` and `rb_capture_stop` exist.

## Task 3: run the stages from `BOT-RUN-SHEET.md`

- **Stage 1 (`prompt_dodge`)** settles the three risks (does BakkesMod record while RLBot runs; does a bot set state; are packets 120 per second). For each, record a clear pass or fail with the evidence: the first captured car frame's position against the scenario's `location`, the `--against` header line (start frame and lag), and whether position error drifts. Also compare the first frame's rotation with the scenario's (the roll sign is the likeliest wrong convention) and run the scenario twice and diff to measure run-to-run noise.
- If Stage 1 passes, continue with Stage 2, one capture per scenario, naming captures `replays\<scenario>.jsonl`. After each run, score it with `rb-verify --scenario tools\rb_tape_bot\scenarios\<scenario>.json --against replays\<scenario>.jsonl` and note position error (mean, max, first over 10 uu and 100 uu), input mismatches, and lag.
- If a stage cannot pass because the game, RLBot or BakkesMod will not cooperate, stop there. A written, evidenced failure is a valid result.

## Task 4: write it down

1. Update `tools/rb_tape_bot/README.md`: the real install and run steps that worked, the status line ("never run against the game" becomes what actually happened), and each open risk marked confirmed or refuted.
2. Update `docs/research/BOT-CAPTURE-PLAN.md` ("Risks to settle first") and `docs/research/BOT-RUN-SHEET.md` with the results, and add a results table (scenario, lag, position error mean and max, input mismatches, and whether the port's prediction in the README table held).
3. If a result refutes an assumption in the code (for example the 1.25 s second-jump window, or the roll sign), do not fix the physics here: record it in `docs/research/RESEARCH-BACKLOG.md` as a new open item with the evidence and the command to reproduce.
4. Write one ADR (`docs/adr/`, next free number, template `docs/adr/TEMPLATE.md`) recording the working RLBot + BakkesMod setup and any choice you had to make (launch order, crate version, how the tape path is passed).
5. Run the repository gates listed above. Commit on a new branch (not `main`) with clear messages ending in the attribution lines in your system reminder. Do not push and do not open a PR; report the branch name.

## Report format

Lead with "Next action: pick a/b/c below (about 1 min)". Then at most five numbered steps saying what ran, the stage reached, the key numbers, and anything that failed with its exact output. End with options a/b/c, offering the most useful next stages or fixes. State plainly what was not verified.
