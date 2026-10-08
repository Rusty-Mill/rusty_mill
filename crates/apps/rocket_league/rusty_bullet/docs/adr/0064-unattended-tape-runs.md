# ADR-0064: Unattended tape runs (job-file trigger, runner, focus finding)

- Status: Accepted (validated 2026-10-06: all eleven scenarios, twice, unattended)
- Date: 2026-10-06
- Deciders: baileyrd
- Related: ADR-0059, RB-RESEARCH-O005, O008, O009, O011,
  `docs/research/BOT-RUN-SHEET.md` (session 2), `tools/rb_tape_bot/README.md`
- Supersedes/Superseded by: none (refines ADR-0059's "operator keeps the game
  focused" consequence and its GUI start-match step)

## Context

ADR-0059 got the tape bot running, but each scenario needed a person: click
Start Match in the RLBot GUI, press F6 and type `rb_capture_start` and
`rb_capture_stop`, and keep the game window focused (session 1 believed
Rocket League pauses freeplay when unfocused). Eleven scenarios, and a second
run of each for a noise floor, made that slow and error-prone. BakkesMod
console commands cannot be driven from outside the game.

## Decision drivers

- Nobody at the machine for a batch; one command.
- No new dependency, nothing outside the repository except what BakkesMod
  needs (the DLL and a `plugins.cfg` line, both done with the owner's consent
  and backups).
- Reuse the plugin's existing start/stop code; keep the console commands.
- Offline freeplay only; nothing touches Easy Anti-Cheat.

## Considered options

1. Drive the RLBot GUI and the game window with synthetic input.
2. BakkesMod's `rconplugin` (a websocket for console commands).
3. A job file the capture plugin polls, plus a runner that talks to RLBot
   core's socket directly.

## Decision

Option 3.

- **Focus:** four manual `prompt_dodge` tapes ran with the game minimised and
  unfocused at 119.6 frames per second with no gap over 0.067 s, scoring the
  same as focused runs; the batches then ran minimised throughout. Focus is
  not needed. The runner times a tape by counting physics frames from core's
  packets, never by wall clock.
- **Plugin 1.4:** polls `<BakkesMod data>/rusty_bullet_capture/job.json` once
  a second through `SetTimeout`; `{"start": "<abs path>"}` and
  `{"stop": true}` call the same `startCapture` and `stopCapture` as the
  console commands; the file is deleted after it runs (the acknowledgement)
  and `heartbeat.txt` (`version`, `capturing`) is rewritten each poll. A
  shared "alive" flag stops a pending poll after unload. The JSON is parsed
  by two small helpers, not a library.
- **Runner `rb_run_tapes`** (second binary in `tools/rb_tape_bot`, no new
  dependency): `CustomBot` carries `run_command`, `root_dir` and `RB_TAPE`
  in the `MatchConfiguration`, so no GUI and no `bots/*.bot.toml`. It starts
  core if absent, launches the game with a warm-up match if the plugin
  heartbeat is missing, starts the capture before the match (as the manual
  runs did), detects the state set from game packets (car within 300 uu of
  the scenario start in the match's first 300 frames), records `total_ticks`
  plus 180 frames, stops capture and match, and on any failure stops what it
  started and exits non-zero.
- **`run_batch.ps1`** builds, runs, and scores each capture with `rb-verify`
  into `results.md` (one row per scenario, per-run values, a repeatability
  flag against a 5 uu mean / 20 uu max run-to-run bound).

## Consequences

### Positive

- Eleven scenarios twice in about five minutes, repeatable by anyone with
  BakkesMod running; a run-to-run noise floor per scenario for free.
- The manual GUI path stays as the fallback.
- Surfaced `corner_slide`'s non-repeatability (O011), which one manual
  capture could not.

### Negative / tradeoffs

- The plugin must load at game start (a `plugins.cfg` line outside the repo)
  and the installed DLL is a manual copy.
- The runner depends on core's `MatchConfiguration` and on `Stadium_P`
  being the arena of the scenarios (the manual runs used the GUI's default
  map, not recorded).
- Repeatability is a proxy (error against the port per run), not a direct
  run-against-run comparison.
- The match looks like a training pack (owner's observation), which fits
  core's freeplay launch (`Playtest` game with the `Freeplay` tag, identical
  for every match); it was not compared with a GUI-started match.

## Validation and revisit triggers

- Validated 2026-10-06: two batches of 22 captures, eleven rows, start frame
  0.0 uu from each scenario's start, ten of eleven repeatable.
- Revisit if a core release changes `MatchConfiguration` or the freeplay
  launch, if a capture ever shows a gap over 0.1 s (a real pause), or if
  the match type matters for results (compare a GUI-started and a
  runner-started capture of one scenario).
