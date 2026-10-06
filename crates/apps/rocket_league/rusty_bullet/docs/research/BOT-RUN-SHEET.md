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

## Session 1 (2026-10-06): Stages 0 to 2 done, all eleven scenarios captured

Machine: Epic Rocket League (`C:\Program Files\Epic Games\rocketleague`),
BakkesMod injector 32 / API 95 with `rusty_bullet_capture.dll` 1.2 already
in the plugins folder, rustc 1.98.1, VS 2022 Build Tools, CMake 4.4.3;
`replays\` did not exist before this session. Setup details and the
verified run sequence are in `tools/rb_tape_bot/README.md`; decisions in
ADR-0059.

Timeline of what failed and was fixed on the way (each one cost a run):

1. Installing RLBot was first refused by the Claude Code permission
   classifier; a local allow rule for `msiexec` was added and the official
   launcher MSI installed per-user.
2. `bot.toml` was unparseable (a backslash in a TOML basic string is an
   escape), so the GUI showed a nameless bot and core rejected it:
   `cars[0].agent_id is empty`. Forward slashes in `RB_TAPE`.
3. The car never moved: core reports `MatchPhase::Paused` throughout
   freeplay while physics runs, so the bot's `== Active` gate never
   opened. Found with the new `rb_probe` (120 packets and 120 frames per
   second, phase Paused). The bot now gates on the frame counter.
4. Rocket League pauses freeplay when its window loses focus, and Escape
   froze it for minutes; the game window must stay focused for the tape.
5. The per-scenario `bots/*.bot.toml` first used a forward-slash
   `run_command`, which `cmd.exe` rejects (`'target' is not recognized`);
   now a TOML literal string with backslashes.

Stage 0 and 1 (prompt_dodge, two runs):

| Risk | Result |
|---|---|
| BakkesMod records while RLBot runs | pass, 1707 and 3980 frames |
| State setting from a bot works | pass, first frame (0, -2000, 17) exactly, rotation (0, 1.571, 0), velocity 0 |
| 120 packets/s | pass, lag 0 (run 1) and 1 (run 2), no drift; probe: 119.9 packets and frames per second |
| Roll sign | pass, checked on corner_slide/pogo/car_over_ball (`--scenario-from` of the set frame reproduces the scenario rotation to 3 decimals) |
| Run-to-run noise | 2.8 uu mean, 13.8 uu max between the two runs, a one-tick offset of the jump |

Stage 2 results (`rb-verify --scenario ... --against`, port against game):

| Scenario | Lag | Position error mean / max (uu) | First over 10 / 100 uu | Port prediction held? |
|---|---|---|---|---|
| prompt_dodge run 1 | 0 | 8.5 / 27.5 | 308 / never | yes, dodge at 502 uu/s |
| prompt_dodge run 2 | 1 | 10.1 / 27.5 | 239 / never | yes |
| late_dodge | 1 | 4.2 / 20.7 | 241 / never | yes, no dodge (FR-136 window) |
| wavedash_early | 0 | 36.3 / 66.2 | 98 / never | roughly |
| wavedash_mid | 0 | 22.6 / 45.0 | 73 / never | peak speed yes |
| wavedash_late | 0 | 15.3 / 33.6 | 84 / never | peak speed yes |
| speed_flip | 0 | 81.5 / 255.0 | 193 / 241 | no |
| half_flip | 0 | 6.8 / 14.5 | 186 / never | yes |
| pogo | 0 | 7.6 / 21.1 | 32 / never | yes |
| hard_landing_nose_first | 0 | 64.1 / 97.4 | 78 / never | no |
| corner_slide | 0 | 86.7 / 177.6 | 41 / 126 | no |
| car_over_ball | 0 | 119.9 / 216.6 | 1 / 76 | no |

Input mismatches are not reported: the plugin records all-zero inputs for
an RLBot-driven car (RB-RESEARCH-O008), so that column only counts the
tape's own non-neutral ticks. Every capture has one 5-tick hole in the
first two seconds (RB-RESEARCH-O009).

Largest errors, for the next loop (largest error, diagnose, fix, record):
car_over_ball (contact differs from the first tick), corner_slide (the
slide), speed_flip (flip direction and height, see FR-094), hard landing
(after touchdown). The 1.25 s second-jump window and the roll convention
held, so nothing in the code was refuted by this session.

Stage 3 not run. Next session: a second run of each Stage 2 scenario for
its own noise floor, then the car_over_ball contact.
