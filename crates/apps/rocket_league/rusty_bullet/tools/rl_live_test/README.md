# rl_live_test

Windows PowerShell scripts that run `rb_rl_bridge` against the real Rocket League (Epic, offline
Exhibition) and write a scrubbed report. They are tooling for a person at the machine, not part of
CI: they start the game, move the mouse and press keys.

```text
cargo build --release -p rb_rl_game_bridge -p rb_rlbot_bridge --bins
pwsh -File run-live-test.ps1 [-PlaySeconds 90] [-ThroughMatchEnd] [-HumanPlays] [-NoClickAfterRollover]
```

Findings from the first runs: [`RL-LIVE-TEST-2026-10-12.md`](../../docs/research/RL-LIVE-TEST-2026-10-12.md).

## What a run does

1. Starts `rb_rl_bridge` with two chaser bots (Blue and Orange) and records both directions.
2. Launches the game the way RLBot's `Epic.Windows.cs` does: the Epic silent-launch URL, kill the
   game as soon as a new `Init: Command line:` line appears in `Launch.log`, relaunch with
   `-rlbot ...` and the login arguments from that line. The login code is single use; if the first
   game reaches the menu and logs in, the relaunch's splash never advances.
3. Walks the menus (splash, "not connected to Epic Online" prompt, Play Offline, Exhibition,
   Demolish = ON CONTACT (FF), Create Match), then spectates (or, with `-HumanPlays`, joins Blue
   and holds throttle and boost).
4. Pauses once, leaves the match and creates it again (a map reload), plays on.
5. With `-ThroughMatchEnd` waits for the match to end and roll into a new one; with
   `-NoClickAfterRollover` it watches for `BroadcastResumed` before touching anything.
6. Stops the bots, the game and the bridge, then runs `rb_rl_game_probe --analyze`,
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
- Epic must be signed in. Right after a long idle the launcher's session can go stale and the game
  exits a few seconds after launch with an empty `Launch.log`; sign in again or restart the launcher.
- `MatchStarted` races the Choose Team screen: it has appeared before the team choice and not at
  all, so the script spectates (or joins) either way and then insists on it.
- Offline play only. The scripts never touch Casual, Competitive or any online mode.

## Credentials

The Epic login arguments live only in a local variable inside `Start-RlSession` and are handed to
the game process. The scripts never print, log or write them, not even their names.
