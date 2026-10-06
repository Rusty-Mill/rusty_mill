# Bot run sheet (first real run of `rb_tape_bot`)

One session on the machine with the game, RLBot v5 and BakkesMod. Setup and
risks: `tools/rb_tape_bot/README.md`. Plan: `BOT-CAPTURE-PLAN.md`. Nothing
here has run against the game yet; each stage below settles one open risk
before the next depends on it. Stop at the first failure and bring back the
output. Results of each session are at the end of this file.

## Stage 0: build and load (10 min)

- `cargo build --release` in `tools/rb_tape_bot`; add `bot.toml` in the RLBot
  GUI; `plugin load rusty_bullet_capture`.
- Pass: the bot appears in an offline match and prints its tape length.

## Stage 1: the three risks, with the control scenario (15 min)

Run `prompt_dodge` (`RB_TAPE=scenarios\prompt_dodge.json`), capturing with
`rb_capture_start prompt_dodge.jsonl` / `rb_capture_stop`.

| Risk | Pass |
|---|---|
| BakkesMod records while RLBot runs | the file has frames |
| State setting from a bot works | first car frame is within 5 uu of the scenario `location` |
| 120 packets/s | `rb-verify --scenario scenarios/prompt_dodge.json --against replays/prompt_dodge.jsonl` reports lag 0 or 1, not a drifting error |

Also read the first frame's rotation against the scenario's (roll sign is the
likeliest wrong convention). Run it twice and diff to measure run-to-run
noise; that noise is the floor for every later comparison.

## Stage 2: the open questions (one run each, 5 min each)

| Scenario | Question it answers | Port predicts |
|---|---|---|
| `late_dodge` | is the second-jump window 1.25 s (FR-136)? | no dodge |
| `wavedash_early` / `_mid` / `_late` | landing with a flip | hop back up, then keep about 1350 uu/s |
| `speed_flip`, `half_flip` | flip, cancel and air roll together | see README table |
| `pogo`, `hard_landing_nose_first` | nose landings, jump on contact | see README table |
| `corner_slide`, `car_over_ball` | the recorded residuals, with exact inputs | see README table |

## Stage 3: your own windows (optional)

Cut any 0.25 to a few seconds from a capture and replay it exactly:

```
rb-verify --scenario-from replays/test2.jsonl 8.95 9.2 mine > scenarios/mine.json
```

(input fields and the 120 Hz check are validated; a 20 Hz capture is
refused). Acceptance seen on this cut: lag 0, position error under 2.5 uu,
zero input mismatches.

## What to bring back

For each run: the capture in `replays/` (gitignored), the
`--scenario ... --against` output, and anything the game did that looked
wrong (dropped inputs, a reset, a wrong spawn). Then the loop is: largest
error, diagnose, fix, record in the spec, as usual.

## Session 1 (2026-10-06): stopped before Stage 0, RLBot not installed

Preflight on the Windows machine (Epic Rocket League at
`C:\Program Files\Epic Gamesocketleague`; BakkesMod injector version 32,
API 95, `rusty_bullet_capture.dll` 1.2 of 2026-10-03 already in the plugins
folder and not in `plugins.cfg`; rustc 1.98.1, VS 2022 Build Tools, CMake
4.4.3; `replays\` absent, so no earlier captures on this box):

| Step | Result |
|---|---|
| `cargo build --release -p rb_verify_cli`, `rb-verify --self` | builds, prints usage |
| `cargo build --release` in `tools/rb_tape_bot` | builds (`rlbot` 0.6.0) |
| `rb-verify --scenario tools/rb_tape_bot/scenarios/prompt_dodge.json` | runs; the port dodges to about 440 uu/s forward at tick 480 |
| Install RLBot v5 | **blocked**: the session's tool permissions refused `msiexec /i rlbot-v5-installer.msi /qn` as an unauthorized persistent install; not retried another way |

Stage 0 to 3: not run. No capture exists, so the results table below is
empty by construction; the columns are the ones to fill.

| Scenario | Lag | Position error mean / max (uu) | First over 10 / 100 uu | Input mismatches | Port prediction held? |
|---|---|---|---|---|---|
| prompt_dodge (run 1) | | | | | |
| prompt_dodge (run 2, noise floor) | | | | | |
| late_dodge | | | | | |
| wavedash_early / mid / late | | | | | |
| speed_flip, half_flip | | | | | |
| pogo, hard_landing_nose_first | | | | | |
| corner_slide, car_over_ball | | | | | |

What the session did settle, from the RLBot v5 docs and source rather than
from a run (details and citations in `tools/rb_tape_bot/README.md`,
decision in ADR-0059):

- The install path moved: rlbot.org/v5 links an MSI from `RLBot/launcher`
  that installs a self-updating launcher into `%LOCALAPPDATA%\RLBot5`; the
  launcher downloads `rlbotgui.exe` and `RLBotServer.exe`. The earlier
  "install the GUI" step in the README was out of date.
- `rlbot` 0.6.0 is the newest crate and its FlatBuffers schema is
  wire-identical to core rc17's; no crate bump.
- Core gates `DesiredGameState` only on the match's `enable_state_setting`
  (default on), with no bot/script distinction.
- `RB_TAPE` goes in `bot.toml`'s `[settings.environment]` table, which core
  applies to the bot process; done, set to `prompt_dodge`.
- Launch order: BakkesMod running first, then start the match from the
  RLBot GUI with **Freeplay** ticked; core relaunches the game with
  `-rlbot`. The wiki documents this order for BakkesMod plugins.
- A remaining doubt for Stage 1: core launches the game with
  `RLBot_PacketSendRate=240` while the wiki caps a bot's tick rate at 120.
  If the capture shows the tape playing at double speed, key the tape on
  `match_info.frame_num`.

Next session: run the `msiexec` line in the README by hand, open the
launcher once, then start at Stage 0.
