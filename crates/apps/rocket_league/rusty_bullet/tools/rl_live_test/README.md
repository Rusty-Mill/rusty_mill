# rl_live_test

Windows PowerShell scripts that run `rb_rl_bridge` against the real Rocket League (Epic, offline
Exhibition) and write a scrubbed report. The live entry point is tooling for a person at the machine: it starts the game, moves the mouse
and presses keys. CI runs only `test-safe.ps1`, with synthetic inputs and mocked process operations.

```text
cargo build --release -p rb_rl_game_bridge -p rb_rlbot_bridge --bins
pwsh -File run-live-test.ps1 [-PlaySeconds 90] [-ThroughMatchEnd] [-HumanPlays] [-NoClickAfterRollover]
```

Findings from the first runs: [`RL-LIVE-TEST-2026-10-12.md`](../../docs/research/RL-LIVE-TEST-2026-10-12.md).

## What a run does

1. Starts `rb_rl_bridge` with two chaser bots (Blue and Orange) and records both directions.
2. Launches the game the way RLBot's `Epic.Windows.cs` does: the Epic silent-launch URL, kill the
   game as soon as a new `Init: Command line:` line appears in `Launch.log`, relaunch with
   `-rlbot ...` and the login arguments from that line. A spent single-use login code is an unproven explanation for the author's stuck-splash
   observations; its semantics were not tested. Bootstrap shutdown now requires verified process
   ownership (see Limits).
3. Walks the menus (splash, "not connected to Epic Online" prompt, Play Offline, Exhibition,
   Demolish = ON CONTACT (FF), Create Match), then spectates (or, with `-HumanPlays`, joins Blue
   and holds throttle and boost).
4. Pauses once, leaves the match and creates it again (a map reload), plays on.
5. With `-ThroughMatchEnd` waits for the match to end and roll into a new one; with
   `-NoClickAfterRollover` it watches for `BroadcastResumed` before touching anything.
6. Stops only retained processes owned by this run, then runs `rb_rl_game_probe --analyze`,
   `--census-in/--census-out` and `rb_rl_demo_count`, and writes `report.txt` next to the
   recordings in `%TEMP%\rl-auto-<stamp>`.

`report.txt` holds no display name: the analysis is made with `rb_rl_game_probe --analyze
--no-names` (players are actor ids and teams, so nothing depends on a pattern matching), and a
second pass still replaces any name that is not a bot's with "human" and the bots with
"bot_a"/"bot_b". The recordings and screenshots are **not** scrubbed: they hold the display name,
so scrub them before sharing.

## Limits

- Menu coordinates are fractions of the game window, calibrated at 2560x1440 with the Season 24
  UI. Another resolution or a UI change moves the buttons.
- The desktop must be unlocked and awake; screen capture and input both fail otherwise.
- Epic must be signed in. The author reported exits after a few seconds with an empty `Launch.log`.
  A stale launcher session was an untested hypothesis; signing in again is not a verified remedy.
- The install path is hardcoded to `C:\Program Files\Epic Games\rocketleague\Binaries\Win64`,
  and the log path to `%USERPROFILE%\OneDrive\Documents\My Games\Rocket League\TAGame\Logs`.
  These are machine-specific; inspect `rl-lib.ps1` before running elsewhere.
- Preflight refuses existing game/bot/bridge processes. Cleanup uses only retained native handles,
  never matching process names. Epic URL activation commonly delegates through an existing launcher:
  if a direct child of the retained, still-live launch process cannot be verified, the run aborts,
  leaves the unowned game alone, and requires manual operator cleanup. No assumption that a newly
  appeared process belongs to this run is safe. This restriction may prevent automated Epic startup.
- The watcher uses separate .NET argument entries (including paths with spaces), a readiness file,
  and an exit check before cleanup. A failed watcher marks the timeline incomplete.
- Every analyzer has a passed/failed/skipped status; native nonzero exit codes or missing inputs
  make the run fail. `rb_rl_demo_count` reads bounded frames and aborts on malformed/truncated data,
  so it never claims lifecycle continuity across undecodable bundles. Report events still consume
  memory proportional to the number of lifecycle transitions, not all physics frames.
- `MatchStarted` races the Choose Team screen: it has appeared before the team choice and not at
  all, so the script spectates (or joins) either way and then insists on it.
- Offline play only. The scripts never touch Casual, Competitive or any online mode.

## Credentials

The Epic login arguments live only in a local variable inside `Start-RlSession` and are handed to
the game process. The scripts never print, log or write them, not even their names.

## Safe regression validation

Run `pwsh -NoProfile -File tools/rl_live_test/test-safe.ps1` from `rusty_bullet`, and
`cargo test --locked -p rb_rl_game_bridge --bin rb_rl_demo_count` from the monorepo root.
The script tests parse production scripts but do not dot-source desktop APIs. Process launches,
cleanup, and watcher readiness are mocked; native exit tests start only short `pwsh` exit stubs.
The Rust tests generate synthetic recordings, including decode gaps and truncated frames.
No private recording or real application is required. Windows desktop integration is deliberately
not covered by these checks; author-reported live observations have not been independently replayed.
