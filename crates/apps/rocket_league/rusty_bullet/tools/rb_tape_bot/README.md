# rb_tape_bot

An RLBot v5 bot that replays a scripted input tape from a known start
state, so the BakkesMod capture plugin
(`bakkesmod-plugin/rusty_bullet_capture`) records a repeatable run of one
mechanic. Plan and reasoning: `docs/research/BOT-CAPTURE-PLAN.md`.

Standalone package, not in the root workspace (its own `[workspace]`), so the
`rlbot` dependency stays out of the main build and CI. The scenario format
lives in the workspace crate `crates/rb_scenario`, which this package and
`rb-verify --scenario` share.

Status: **run against the game on 2026-10-06** (RLBot core v5.0.0-rc17,
Epic Rocket League, BakkesMod injector 32): all eleven scenarios captured
and scored, results below and in `docs/research/BOT-RUN-SHEET.md`
(session 1). Setup decisions: ADR-0059.

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

What worked on 2026-10-06 (`RLBot/core` v5.0.0-rc17, `RLBot/gui` beta23,
`RLBot/launcher`, Epic game, BakkesMod injector 32). Where the RLBot docs
disagreed with the earlier text of this file, the docs won and the
difference is noted.

### Install (once)

1. rlbot.org/v5 links an MSI, not "the GUI":
   <https://github.com/RLBot/launcher/releases/download/installer/rlbot-v5-installer.msi>
   (1.7 MB, SHA-256
   `549af50bdaf76a251375e6a6543c5013d1e8fcceca648d2cbc4800ecd553dc5c`
   on 2026-10-06). Per-user WiX install, no elevation; it puts
   `launcher.exe` in `%LOCALAPPDATA%\RLBot5\bin`. Silent form:
   `msiexec /i rlbot-v5-installer.msi /qn /norestart`.
2. Run `launcher.exe` once. It downloads the latest GitHub releases of
   `rlbotgui.exe` (`RLBot/gui`) and `RLBotServer.exe` (`RLBot/core`) into
   the same folder plus the botpack, keeps them updated, and starts core
   and the GUI. `RLBotServer.exe --version` prints core's version.
3. `cargo build --release` in this directory (builds `rb_tape_bot.exe`
   and the `rb_probe.exe` diagnostic).

### Run one scenario

1. Start **BakkesMod first** and leave it waiting. Core kills any running
   Rocket League and relaunches it itself with
   `-rlbot RLBot_ControllerURL=127.0.0.1:<port> RLBot_PacketSendRate=240 -nomovie`
   (for Epic it first starts the game through `com.epicgames.launcher://`
   to read the login arguments from `Launch.log`, then restarts it
   directly). BakkesMod injected into that process and the plugin loaded
   normally; this is the order the RLBot wiki documents for BakkesMod
   plugins (`docs/v5/miscellaneous/lan-setup.md`).
2. In the GUI add this folder as a bot folder. It scans recursively for
   `bot.toml` and `*.bot.toml`, so it lists "RB Tape Bot" (`bot.toml`,
   plays `prompt_dodge`) and one **"RB Tape: <scenario>"** entry per file
   in `bots/`, each with its own `RB_TAPE` in `[settings.environment]`.
   Core sets that table on the bot process after inheriting its own
   environment (`ConfigParser.GetEnvironment`,
   `LaunchManager.ApplyEnvironment`); the GUI parses the files when it
   scans, so refresh the folder after editing one. Put the scenario's
   entry on Blue, nobody else.
3. Match settings: launcher **Epic** (or Steam), Soccar, any map, default
   mutators; extra options **Enable State Setting** (default on) and
   **Freeplay** ticked. Keep "auto start agents" and "wait for agents".
4. In the game, F6: `plugin load rusty_bullet_capture` (once per game
   start), then `rb_capture_start <absolute path>\<scenario>.jsonl`
   (a relative path lands in the game's working directory).
5. Press **Start Match**, then **click into the game window and keep it
   focused** for the tape's length plus a few seconds. Rocket League
   pauses freeplay whenever its window loses focus, and Escape opens a
   pause menu that froze the game for us; while paused core repeats one
   packet 240 times a second with the same frame number, and the bot
   waits. Core's log (and the bot's stdout, which core forwards) shows
   `Spawning RB Tape: <scenario>`, the bot's tape line, and `tape bot:
   physics ticking at frame N (phase Paused), setting the start state`.
6. F6, `rb_capture_stop`. Score the file, see below. The GUI leaves you
   in spectator view because the match has no human; the plugin records
   every car regardless.

Tape lengths (settle plus steps, at 120 Hz): prompt_dodge 4.5 s,
late_dodge 5.6 s, half_flip 3.1 s, speed_flip 3.1 s, pogo 2.7 s, the
rest 2.0 s.

If the bot does nothing: run `target\release\rb_probe.exe [seconds]`
while the match is up. It connects to core as a passive client and prints,
once a second, the match phase, frame number, packets per second and car
0's position, rotation, air state and boost. A frozen frame number means
the game is paused (focus or Escape); 120 packets and 120 frames per
second with the phase `Paused` is normal freeplay.

Core's own log is lost when the launcher starts it (no console). To keep
it, stop `RLBotServer.exe` and start it yourself with stdout redirected to
a file; the GUI reconnects on the next Start Match.

### Facts settled against the running game (2026-10-06)

- **Match phase.** Core reports `MatchPhase::Paused` for the whole
  freeplay session while physics runs at 120 Hz (core rc17 marks a
  freshly loaded map Paused and the bridge never changes it in freeplay).
  The bot therefore gates on the physics frame counter advancing, not on
  `Active`, and indexes the tape by frame.
- **Packet rate.** 120 game packets and 120 physics frames per second
  (7192 packets, 7196 frames in 60 s) despite `RLBot_PacketSendRate=240`.
  While paused, 240 identical packets per second.
- **State setting from a bot.** Works: the first captured frame of every
  run is the scenario's location to the decimal, with the scenario's
  velocity, angular velocity and boost; the ball is placed too. Core gates
  `DesiredGameState` only on the match's `enable_state_setting`
  (`FlatBuffersSession.cs`), no bot/script distinction.
- **Rotation convention.** `corner_slide` start (pitch, yaw, roll) =
  (0.4624, -0.2194, 1.2204) in the scenario was recorded as (0.4621,
  -0.2194, 1.2203) by `rb-verify --scenario-from` on the set frame; `pogo`
  pitch -1.2 and `car_over_ball` yaw -2.2569 likewise. Roll sign is right.
- **Alignment.** `rb-verify --scenario ... --against` found the start in
  every capture with lag 0 or 1.
- **Protocol.** `rlbot` 0.6.0 (newest crate, schema `c38374e`) against
  core rc17 (schema `f90c844`): the `.fbs` diff is comments and one
  `deprecated` attribute; no crate bump needed.
- **Recorded inputs are all zero for the bot's car.** The plugin reads
  `CarWrapper::GetInput()`, which stays neutral when RLBot drives the car,
  so the "recorded inputs that differ from the tape" count only reflects
  the tape's own non-neutral ticks. Backlog item RB-RESEARCH-O008. The
  tape is the input ground truth for these captures.
- **Dropped ticks.** Every capture has one hole of 5 ticks (0.0417 s)
  somewhere in the first two seconds of the tape, and 2 to 7 missing
  ticks at the state-set frame itself. Backlog item RB-RESEARCH-O009.
- **Noise floor.** Two `prompt_dodge` runs: after aligning on the set
  frame, position differs by 2.8 uu mean and 13.8 uu max, almost all of
  it a one-tick offset of the jump (lag 0 vs lag 1).

### Results (2026-10-06, port vs game)

`rb-verify --scenario scenarios/<name>.json --against replays/<name>.jsonl`.
"Held" is the scenario table's port prediction against what the game did.

| Scenario | Lag | Pos. error mean / max (uu) | First over 10 / 100 uu | What the game did | Port prediction held? |
|---|---|---|---|---|---|
| prompt_dodge (two runs) | 0, 1 | 8.5 / 27.5 and 10.1 / 27.5 | tick 308 / never | dodge fires, 502 uu/s forward | yes |
| late_dodge | 1 | 4.2 / 20.7 | 241 / never | no dodge on the 1.6 s press; car lands | yes (1.25 s window, FR-136) |
| wavedash_early | 0 | 36.3 / 66.2 | 98 / never | flips in the air to 1466 uu/s, rises to 152 uu, lands at tick 209 | roughly (hop up), 66 uu off |
| wavedash_mid | 0 | 22.6 / 45.0 | 73 / never | lands tick 28, 1466 uu/s peak, coasts to 436 | peak yes, 45 uu off |
| wavedash_late | 0 | 15.3 / 33.6 | 84 / never | lands tick 30, 1454 uu/s peak, coasts to 441 | peak yes, 34 uu off |
| speed_flip | 0 | 81.5 / 255.0 | 193 / 241 | flip from tick 216; 1913 uu/s at the end, but 550 uu/s velocity error during the flip | no: flip direction and height differ |
| half_flip | 0 | 6.8 / 14.5 | 186 / never | flips, lands at tick 370 | yes |
| pogo | 0 | 7.6 / 21.1 | 32 / never | contact jump to 520 uu/s, lands tick 141 | yes |
| hard_landing_nose_first | 0 | 64.1 / 97.4 | 78 / never | nose landing at 2300 uu/s, settles to 1514 | no: diverges after the landing |
| corner_slide | 0 | 86.7 / 177.6 | 41 / 126 | corner slide, stops | no: diverges in the slide |
| car_over_ball | 0 | 119.9 / 216.6 | 1 / 76 | ball struck immediately (ball to 1870 uu/s) | no: contact differs from tick 1 |

The input-mismatch column of `rb-verify` is omitted: inputs are not
recorded for a bot car (above). Captures live in `replays/` (gitignored).

### Open risks

| Risk | Status (2026-10-06) |
|---|---|
| BakkesMod records while RLBot runs the game | confirmed: eleven captures |
| State setting from a bot is accepted by core | confirmed |
| Packets arrive at 120 per second | confirmed (120 packets, 120 frames per second) |
| Protocol mismatch between crate and core | refuted: schemas wire-identical |
| Roll sign of the start rotation | confirmed correct |
| Run-to-run noise | 2.8 uu mean, 13.8 uu max, dominated by a one-tick jump offset |
| New: inputs not recorded for the bot car | open, RB-RESEARCH-O008 |
| New: one 5-tick hole per capture | open, RB-RESEARCH-O009 |

## Scoring

`rb-verify --scenario scenarios/<name>.json --against <capture> [every]`
lines the capture up with the port's free-run prediction (start found by
the car's start location, lag 0 to 3 ticks) and prints per-tick error, the
first tick the error passes 10 and 100 uu, and how many recorded inputs
differ from the tape (nonzero means the bot did not play the tape as
written). The three reproduction scenarios only follow their source
recording for the first 0.2 to 0.5 s, because the tape does not carry the
rest of the recorded inputs.

`rb-verify --self-onestep <capture>` and `--self-kstep <capture> 30` score
the recording's own one-step and k-step error. These seed from the first
grounded, neutral frame, which an airborne start never has; put
`--seed-first-frame` first (`rb-verify --seed-first-frame --self-kstep
<capture> 30`) to seed from the capture's first frame instead. The hidden
jump state is still assumed neutral at that frame.

## Scenarios from a human performance

`rb-verify --scenario-from <capture> <from-secs> <to-secs> [name] >
scenarios/<name>.json` cuts a scenario from a window of a recording: the
car's and ball's state at the window's first frame as the start (rotation
converted to `[pitch, yaw, roll]`), the recorded inputs as a run-length
tape, no settling. The bot then replays the performance exactly, and
`--scenario <it> --against <new capture>` scores the replay. The capture
must be 120 Hz (mean frame interval within 5%) and must carry inputs, so a
replay-derived capture cannot be cut.
