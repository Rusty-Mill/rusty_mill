# rb_tape_bot

An RLBot v5 bot that replays a scripted input tape from a known start
state, so the BakkesMod capture plugin
(`bakkesmod-plugin/rusty_bullet_capture`) records a repeatable run of one
mechanic. Plan and reasoning: `docs/research/BOT-CAPTURE-PLAN.md`.

Standalone package, not in the root workspace (its own `[workspace]`), so the
`rlbot` dependency stays out of the main build and CI.

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

Shipped:

- `prompt_dodge.json`: jump, wait 0.5 s, dodge. A dodge must fire (control).
- `late_dodge.json`: jump, wait 1.6 s, dodge. Past the 1.25 s window the
  port expects no dodge (`RB-PHYSICS-001-FR-136`); the capture settles it.

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

`rb-verify --self-onestep <capture>` and `--self-kstep <capture> 30`. An
airborne start has no grounded, neutral frame, which `rb-verify` currently
needs to seed from; settle the car on the floor first (`settle_ticks`), as
the shipped scenarios do, until a `--seed-first-frame` option exists.
