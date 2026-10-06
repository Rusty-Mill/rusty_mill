# rb_tape_bot

An RLBot v5 bot that replays a scripted input tape from a known start
state, so the BakkesMod capture plugin
(`bakkesmod-plugin/rusty_bullet_capture`) records a repeatable run of one
mechanic. Plan and reasoning: `docs/research/BOT-CAPTURE-PLAN.md`.

Standalone package, not in the root workspace (its own `[workspace]`), so the
`rlbot` dependency stays out of the main build and CI. The scenario format
lives in the workspace crate `crates/rb_scenario`, which this package and
`rb-verify --scenario` share.

Status: builds, scenario reader tested; **never run against the game**.

## Scenario files (`scenarios/*.json`)

```json
{
  "name": "...",
  "settle_ticks": 240,
  "car":  { "location": [x, y, z], "rotation": [pitch, yaw, roll],
            "velocity": [x, y, z], "angular_velocity": [x, y, z], "boost": 0 },
  "ball": { "location": [x, y, z], "velocity": [x, y, z] },
  "steps": [ { "ticks": 12, "jump": true },
             { "ticks": 1, "jump": true, "pitch": -1 } ]
}
```

Every car and ball field is optional (left as the game has it); step inputs
default to neutral (`throttle`, `steer`, `pitch`, `yaw`, `roll`, `jump`,
`boost`, `handbrake`). Ticks are game packets, assumed 120 per second. On the
first active packet the bot sets the state; the tape starts on the next one,
after `settle_ticks` neutral packets. Rotation is radians in RLBot's
convention (yaw 0 faces +x). Pitch -1 is nose down (a forward dodge).

Shipped scenarios. "Port predicts" is what `rb_physics_bullet` does with the
same start and tape, reproducible with
`rb-verify --scenario scenarios/<name>.json` from the repository root; the
capture is the test of that prediction.

| File | What it does | Port predicts | Targets |
|---|---|---|---|
| `prompt_dodge` | full-height jump, forward dodge 0.5 s after the press | the dodge fires (about 500 uu/s forward) | control for the next row |
| `late_dodge` | full-height jump, dodge 1.6 s after the press (1.4 s after the hold ends) | no dodge (1.25 s window, FR-136) | second-jump window |
| `wavedash_early` / `_mid` / `_late` | nose-up car falling at 900 uu/s forward, forward flip 15, 19 or 23 ticks in | early: hops back up; mid and late: land and keep about 1350 uu/s | landing with a flip, tire grip, wheel pushback |
| `speed_flip` | kickoff: boost, jump, nose up, diagonal flip, cancel pitch, air roll | flips to about 1100 uu/s forward-side, 70 uu up | flip torque, cancel, boost together |
| `half_flip` | car facing -y moving +y, jump, back flip, cancel, air roll | keeps spinning (pitch spin about -5 rad/s) for the flip window | flip cancel rule, air roll |
| `pogo` | drop nose-first from 260 uu, jump on contact, rotate nose down | the contact jump pushes the car along its own up axis (about +270 uu/s forward) and it lands again | hard nose landings, jump on contact |
| `hard_landing_nose_first` | the `test2.jsonl` 18.308 s state, no input | the recorded landing (rebounds, then rests) | the 18.35 s residual, with exact inputs |
| `corner_slide` | the `test2.jsonl` 8.95 s state and inputs | hits the corner and slides | the 8.958 s and 9.125 s residuals |
| `car_over_ball` | the `hitjump.jsonl` 77.642 s state, ball resting | wheels over the ball | the 77.667 s ball residual |

The last three start from states read out of the owner's captures, so the new
capture can be compared directly with the old one. Rotations were converted
from the capture's quaternions to RLBot's pitch/yaw/roll and checked by
converting back (the 18.308 s quaternion round-trips to four decimals); the
roll sign is the convention most likely to be wrong in the game, so check the
first captured frame's rotation against the scenario's.

## Running (on the machine with the game)

1. Install RLBot v5 (the GUI from https://rlbot.org/v5/).
2. `cargo build --release` in this directory.
3. Set `RB_TAPE` to a scenario path (for example
   `scenarios\late_dodge.json`) in the environment RLBot starts bots with.
4. In the RLBot GUI add this folder's `bot.toml` as a bot on a team, start
   an offline match, and in BakkesMod: `plugin load rusty_bullet_capture`,
   `rb_capture_start late_dodge.jsonl`, then `rb_capture_stop` when the tape
   ends (the bot's tape length is printed at start).
5. Copy the capture to `replays/` (gitignored) and score it, see below.

Open risks (unconfirmed): that BakkesMod records while RLBot runs the game;
that state setting from a bot (rather than a script) is accepted by core;
that packets arrive at 120 per second. Run each scenario twice and diff to
measure run-to-run noise.

## Scoring

`rb-verify --scenario scenarios/<name>.json --against <capture> [every]`
lines the capture up with the port's free-run prediction (start found by
the car's start location, lag 0 to 3 ticks) and prints per-tick error, the
first tick the error passes 10 and 100 uu, and how many recorded inputs
differ from the tape (nonzero means the bot did not play the tape as
written). The three reproduction scenarios only follow their source
recording for the first 0.2 to 0.5 s, because the tape does not carry the
rest of the recorded inputs.

`rb-verify --self-onestep <capture>` and `--self-kstep <capture> 30`. An
airborne start has no grounded, neutral frame, which `rb-verify` currently
needs to seed from; settle the car on the floor first (`settle_ticks`), as
the shipped scenarios do, until a `--seed-first-frame` option exists.
