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

Stage 2 results (`rb-verify --scenario ... --against`, port against game,
re-scored after RB-VERIFY-003 0.23.0 aligned recorded frames by timestamp;
the first scoring aligned by row and every capture hole became a permanent
offset, which made prompt_dodge read 8.5 uu mean and car_over_ball look
wrong from tick 1):

| Scenario | Lag | Position error mean / max (uu) | First over 10 / 100 uu | Port prediction held? |
|---|---|---|---|---|
| prompt_dodge run 1 | 1 | 1.2 / 3.6 | never / never | yes, dodge at 502 uu/s |
| prompt_dodge run 2 | 1 | 1.2 / 3.6 | never / never | yes |
| late_dodge | 2 | 1.8 / 4.7 | never / never | yes, no dodge (FR-136 window) |
| wavedash_early | 0 | 14.2 / 25.3 | 98 / never | roughly |
| wavedash_mid | 0 | 4.1 / 5.1 | never / never | yes |
| wavedash_late | 0 | 4.1 / 5.1 | never / never | yes |
| speed_flip | 0 | 94.4 / 289.3 | 218 / 237 | no |
| half_flip | 0 | 4.3 / 13.9 | 320 / never | yes |
| pogo | 0 | 2.4 / 4.3 | never / never | yes |
| hard_landing_nose_first | 0 | 10.0 / 22.0 | 125 / never | yes, within 22 uu |
| corner_slide | 0 | 100.3 / 177.6 | 41 / 95 | no |
| car_over_ball | 1 | 156.2 / 290.5 | 2 / 71 | no |

Input mismatches are not reported: the plugin records all-zero inputs for
an RLBot-driven car (RB-RESEARCH-O008), so that column only counts the
tape's own non-neutral ticks. Every capture has one 5-tick hole in the
first two seconds and 2 to 7 missing ticks at the set frame
(RB-RESEARCH-O009); `rb-verify` now skips them.

The three that diverge, with the evidence for the next loop:

- **car_over_ball** (RB-RESEARCH-O010): the port's first contact tick is
  right (port tick 3 at (43.9, 112.0, 163.1), velocity (-1211, -1447,
  400); game (43.9, 112.1, 163.2), (-1210, -1443, 405)), but the game's
  car keeps gaining upward speed for four more ticks (405, 444, 475, 498
  uu/s) and loses more horizontal speed (to -1138, -1311) while the
  port's contact is over in one tick (400, 392, 387; -1214, -1452). The
  game's hit carries about 1.6 times the port's impulse, spread over
  about four ticks.
- **corner_slide**: agrees to 5 uu for 36 ticks, then the game's car
  climbs the corner higher (z 326 uu at tick 84 against the port's 263)
  and stays up longer; 100 uu by tick 96. The 9.125 s residual of
  `test2.jsonl`, now with exact inputs.
- **speed_flip**: agrees to 10 uu until the flip at tick 216, then 550
  uu/s of velocity error through the flip and 289 uu by the end of it;
  the diagonal dodge with pitch cancel and air roll goes a different way
  (see RB-PHYSICS-001-FR-094, the dodge-direction miss).

Nothing in the code was refuted by this session: the 1.25 s second-jump
window and the roll convention held, and the nose landing is within 22 uu.

Stage 3 not run. Next session: a second run of each Stage 2 scenario for
its own noise floor, then the car_over_ball contact.

## Session 2, stage 1 (partial, 2026-10-06): focus and plugin 1.3

Not the controlled four-way test the plan asks for; one uncontrolled run
that happens to answer part of it. Plugin 1.3 (built and installed this
session). The operator started the capture in the game, then went to the
RLBot GUI and pressed Start Match four times about 10 s apart, leaving the
GUI in front each time, and returned to the game to stop the capture. So
The operator confirmed afterwards that the game window was **minimised to
the taskbar** (and the GUI focused) for all four tapes. One capture, `replays/v13_prompt_dodge.jsonl`
(gitignored), 11047 frames in 92.3 s (119.6 per second); cut at the four
state-set teleports into `v13_run1..4.jsonl`.

| Run | Frames compared | Lag | Pos. error mean / max (uu) | Input mismatches | Dodge fired |
|---|---|---|---|---|---|
| 1 | 543 | 1 | 1.2 / 3.6 | 0 | yes |
| 2 | 542 | 1 | 1.2 / 3.6 | 0 | yes |
| 3 | 542 | 1 | 1.2 / 3.6 | 0 | yes |
| 4 | 541 | 2 | 2.2 / 5.5 | 3 | yes |

- Largest gap between frames in the whole capture: 0.067 s (8 ticks), the
  known 5-tick hole at each state-set (RB-RESEARCH-O009). No pause, no stall.
- **Focus:** with the game window behind the GUI, tapes ran at full rate and
  matched the focused session-1 results (lag 0 to 2, error inside the 2.8 uu
  noise floor). So "the window must stay focused" (session 1) is not
  supported: a minimised, unfocused game ran the tape normally.
  **Not tested:** a second monitor, `rb_probe` alongside, and the window
  state is not recorded in the capture, so it rests on the operator's
  account. Only `prompt_dodge`, the shortest tape, was run this way. Session 1's freeze
  may have been the Escape pause menu.
- **Plugin 1.3 (RB-RESEARCH-O008):** the recorded inputs are non-zero and
  match the tape (jump for 24 frames, then once 0.5 s later); runs 1 to 3
  have zero mismatches. Run 4's jump came one tick early (lag 2, three
  mismatches), the same one-tick jitter as the session-1 noise floor. The
  five `bakkesmod.log` diagnostic lines show all-zero from both sources
  because they are logged before the tape starts; they do not test the fix.

## Session 2, stage 2 (2026-10-06): unattended batch of all eleven scenarios

One command from the repository root, nobody touching the machine:
`powershell -File tools\rb_tape_bot\run_batch.ps1` (ADR-0064). Plugin 1.4
(job-file trigger) installed with the 1.3 DLL kept as `.1.3.dll.bak`, and
`plugin load rusty_bullet_capture` added to BakkesMod's `plugins.cfg`
(backup `plugins.cfg.bak`). Final batch `replays/batch_20261006-195814/`
(gitignored), eleven scenarios, two runs each, results table below; the
script's own `results.md` is in that folder.

| Scenario | Lag | Pos. error mean / max (uu), run 1; run 2 | First over 10 / 100 uu | Frames | Largest gap (s) | Repeatable |
|---|---|---|---|---|---|---|
| car_over_ball | 1; 1 | 155.7 / 290.5; 155.7 / 290.5 | 2 / 71 | 470; 471 | 0.050 | yes |
| corner_slide | 1; 0 | 27.7 / 45.0; 79.7 / 146.3 | 2 / never; 45 / 118 | 471; 470 | 0.050 | **no** |
| half_flip | 0; 0 | 4.3 / 13.9; 4.3 / 13.9 | 320 / never | 590; 591 | 0.067 | yes |
| hard_landing_nose_first | 0; 0 | 7.4 / 13.8; 11.3 / 23.1 | 170 / never; 109 / never | 470; 470 | 0.058 | yes (d mean 3.9) |
| late_dodge | 2; 2 | 1.8 / 4.7; 1.8 / 4.7 | never / never | 950; 952 | 0.075 | yes |
| pogo | 0; 0 | 2.4 / 4.3; 2.4 / 4.3 | never / never | 591; 590 | 0.050 | yes |
| prompt_dodge | 3; 3 | 4.0 / 10.7; 4.0 / 10.7 | 452 / never | 832; 833 | 0.050 | yes |
| speed_flip | 2; 3 | 87.4 / 273.7; 85.2 / 266.8 | 219 / 238; 209 / 239 | 591; 592 | 0.050 | yes (d mean 2.2) |
| wavedash_early | 0; 0 | 14.2 / 25.3; 14.3 / 25.3 | 98 / never | 469; 471 | 0.058 | yes |
| wavedash_late | 0; 0 | 4.1 / 5.1; 4.1 / 5.1 | never / never | 469; 471 | 0.067 | yes |
| wavedash_mid | 0; 0 | 4.1 / 5.1; 4.1 / 5.1 | never / never | 470; 471 | 0.050 | yes |

- **Acceptance:** one command, eleven rows, and in all 22 captures the
  frame `rb-verify` aligns as the start is 0.0 uu from the scenario's start
  location. The capture also holds a few frames before that (the match's
  kickoff spawn), so "first frame of the file" is the spawn, not the start.
- **Repeatability:** ten of eleven agree between two runs to under 4 uu mean:
  eight to 0.1 uu or better (car_over_ball, pogo, the three wavedashes,
  half_flip, late_dodge, prompt_dodge), speed_flip to 2.2, hard_landing to 3.9. **corner_slide is not repeatable:** the same tape from
  the same start gave 27.7 / 45.0 and 79.7 / 146.3 uu against the port in
  this batch, 19.5 / 33.1 and 100.5 / 177.6 in an earlier batch the same
  day (session 1: 100.3 / 177.6). The cars differ from tick 12 (speed 1160 vs
  1233 uu/s), i.e. at the first corner-wall contact, so the corner hit is
  sensitive to something the tape and start state do not fix (sub-tick
  phase of the state set). One capture of it is therefore not a ground truth
  at the 100 uu level. RB-RESEARCH-O011.
- **corner_slide, ten more runs** (`batch_20261006-200506`): mean error 16.6
  to 106.5 uu, spread 89.9 / 166.5. The outcomes cluster in three groups by
  corner peak height (288 to 289, 312, 326 to 331 uu), several runs
  bit-identical, so it is deterministic in some discrete variable; the delay
  from the state set to the first steer input (1 to 4 ticks) is a candidate
  but does not fully explain it. RB-RESEARCH-O011 has the detail. The
  script's repeatability check compared only runs 1 and 2 (it said "yes"
  here); it now uses the spread over all runs.
- **corner_slide experiments** (six runs each, `batch_20261006-201038`):
  the car started 0.5 s earlier (`experiments/corner_slide_far.json`) is
  bit-identical in all six runs (port error 23.0 / 41.3 uu every time);
  the original start with 24 neutral ticks first (`corner_slide_padded`)
  still splits into three outcomes (up to 93 uu apart). The variation comes
  from the state set next to the wall, not from input timing. See O011.
- **Session 1's three divergences stand:** car_over_ball (155.7 mean against
  156.2), speed_flip (85 to 87 against 94.4) and the higher corner_slide runs
  (79.7 to 100.5 against 100.3) reproduce, so they stay the physics targets
  (corner_slide with O011's caveat).
- **Lag** varies 0 to 3 ticks between scenarios and, for prompt_dodge, from
  1 (session 1) to 3 (this batch) with the same tape, with identical error
  between its two runs here; the aligner absorbs it.
- **Capture holes (O009):** every capture still has holes, the largest
  0.075 s (9 ticks, late_dodge), most 0.050 to 0.067 s. Reported, not
  hidden; `rb-verify` skips them.
- **Failures on the way** (each cost a run): the first runner version
  detected the bot's start by distance under 40 uu, which a car starting at
  1600 uu/s leaves in one tick (`hard_landing_nose_first` timed out after
  120 s; the tolerance is now 300 uu); detection also fired on the previous
  match's last packets when it ended near the next start (pogo then
  prompt_dodge), recording 24 s instead of 5 s, so it now only accepts the
  match's first 300 frames; a log file left open by an earlier shell killed
  one launch of the command. The scoring script first failed to parse
  `tick N` in "first over" and reported the proximity of the first table row
  as the start error (18 uu for car_over_ball, which is already in contact).
- **Match type:** the owner first reported the match looking like a private
  match, then corrected it: it looked like a training pack. That fits core's
  freeplay launch (`Open Stadium_P?game=TAGame.GameInfo_Soccar_TA?Playtest?GameTags=PlayerCount8,Freeplay,UnlimitedTime`,
  the same for every match) and the plugin, which is a freeplay-type plugin,
  recorded in every one. Not compared against a GUI-started match.
- **Not verified:** Second-monitor focus and `rb_probe` alongside were never run.
