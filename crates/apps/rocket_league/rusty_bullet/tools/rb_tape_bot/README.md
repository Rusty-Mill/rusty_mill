# rb_tape_bot

An RLBot v5 bot that replays a scripted input tape from a known start
state, so the BakkesMod capture plugin
(`bakkesmod-plugin/rusty_bullet_capture`) records a repeatable run of one
mechanic. Plan and reasoning: `docs/research/BOT-CAPTURE-PLAN.md`.

Standalone package, not in the root workspace (its own `[workspace]`), so the
`rlbot` dependency stays out of the main build and CI. The scenario format
lives in the workspace crate `crates/rb_scenario`, which this package and
`rb-verify --scenario` share.

Status: builds (`rlbot` 0.6.0, the newest published version, wire-compatible
with RLBot core v5.0.0-rc17, see below); **never run against the game**. The
first run session (2026-10-06, `docs/research/BOT-RUN-SHEET.md`) stopped at
installing RLBot: the session's tool permissions refused to run the
installer, so Stage 0 onwards still waits for the owner to install it by
hand (one command, below).

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

Checked against the RLBot v5 docs and source on 2026-10-06 (`RLBot/core`
master = v5.0.0-rc17 of 2026-08-15, `RLBot/gui` beta23, `RLBot/launcher`,
`RLBot/wiki` `docs/v5`); where they disagreed with the earlier text of this
file, the docs won and the difference is noted.

### Install (once)

1. The official download is no longer "the GUI": rlbot.org/v5 now links an
   MSI, <https://github.com/RLBot/launcher/releases/download/installer/rlbot-v5-installer.msi>
   (1.7 MB, SHA-256
   `549af50bdaf76a251375e6a6543c5013d1e8fcceca648d2cbc4800ecd553dc5c`
   on 2026-10-06). It is a per-user WiX install with no elevation: it
   puts `launcher.exe` in `%LOCALAPPDATA%\RLBot5\bin`, plus a Start-menu
   and desktop shortcut. Silent form:
   ```
   msiexec /i rlbot-v5-installer.msi /qn /norestart
   ```
2. Run `launcher.exe` once (the shortcut "RLBot v5"). It downloads the
   latest GitHub releases of `rlbotgui.exe` (`RLBot/gui`) and
   `RLBotServer.exe` (`RLBot/core`) into `%LOCALAPPDATA%\RLBot5`, keeps
   them updated, and starts the GUI.
3. `cargo build --release` in this directory. `RLBotServer.exe --version`
   prints core's version; compare it with the compatibility note below if
   anything fails to connect.

### Run one scenario

1. Start **BakkesMod first** and leave it waiting. Core kills any running
   Rocket League and relaunches the game itself with
   `-rlbot RLBot_ControllerURL=127.0.0.1:<port> RLBot_PacketSendRate=240 -nomovie`
   (for Epic it first starts the game through
   `com.epicgames.launcher://` to read the login arguments from
   `Launch.log`, then restarts it directly), so BakkesMod must already be
   running to inject into the process core creates. This is the order the
   RLBot wiki itself documents for BakkesMod plugins ("Make sure that
   BakkesMod is running. Start Rocket League with RLBotGUI",
   `docs/v5/miscellaneous/lan-setup.md`).
2. Set the tape: edit `RB_TAPE` in `bot.toml`'s `[settings.environment]`
   table (for example `scenarios\late_dodge.json`). Core sets every key of
   that table on the bot process after inheriting its own environment
   (`ConfigParser.GetEnvironment`, `LaunchManager.ApplyEnvironment`), so a
   value in `bot.toml` beats a variable exported in the shell that
   started the GUI. The bot process runs from `root_dir`, which defaults
   to this folder, so relative paths work.
3. In the GUI: add this folder as a bot folder (it scans for `bot.toml`
   and `*.bot.toml`), put "RB Tape Bot" on one team, no opponents. In the
   match settings' extra options tick **Enable State Setting** (default
   on; match field `enable_state_setting`) and **Freeplay** (field
   `freeplay`; the wiki: "allows the players to use training keybinds,
   Bakkesmod plugins, and other features that are only allowed in free
   play"). Launcher: Epic on the owner's machine (the game is
   `C:\Program Files\Epic Games\rocketleague`). No mutators.
4. Start the match. The bot prints `tape bot: '<name>' (<n> ticks)` in
   its console window; a missing `RB_TAPE` panics with "set RB_TAPE to
   the scenario JSON path".
5. In BakkesMod (F6): `plugin load rusty_bullet_capture` (once per game
   start; the plugin is not in `plugins.cfg`), `rb_capture_start
   <absolute path>\<scenario>.jsonl`, then `rb_capture_stop` after the
   tape length has elapsed (n ticks at 120 per second). A relative path
   is written under the game's working directory.
6. Copy the capture to `replays\<scenario>.jsonl` (gitignored) and score
   it, see below.

Scripted alternative to the GUI clicks: core only needs a match
configuration over its socket (`RLBotServer.exe [sockets-port]
[game-port]`, default 23234), as the crate's `start_match` example does
with `MatchConfiguration { enable_state_setting: true, .. }`. Not built
here; the GUI path is enough for one session.

### Compatibility (checked 2026-10-06)

- `rlbot` 0.6.0 (crates.io, 2026-09-21) is the newest release, pinned to
  `flatbuffers-schema` `c38374e`. Core rc17 pins `f90c844` ("Move
  ConsoleCommand to end of union", 2026-08-08). `git diff c38374e f90c844
  -- '*.fbs'` is two comment lines and the `deprecated` attribute on
  `DesiredGameState.console_commands`: the wire format is identical, so
  no crate bump is needed.
- Core accepts `DesiredGameState` from any connected agent when the match
  has `enable_state_setting` (`FlatBuffersSession.cs`); bots and scripts
  are not distinguished. So "does core accept state setting from a bot"
  is answered by the source: yes, if the match setting is on.
- Core's `RLBOT_SERVER_PORT` and `RLBOT_AGENT_ID` are what
  `AgentEnvironment::from_env` reads; `agent_id` in `bot.toml` must match
  the bot's (`rusty_bullet/tape_bot`, it does).
- Packet rate: core launches the game with `RLBot_PacketSendRate=240`,
  and the wiki says a bot's tick rate is `min(game FPS, 120)`. The tape
  assumes one packet per 120 Hz tick; check the first capture's lag and
  drift (Stage 1) before trusting it, and if packets repeat, key the tape
  on `match_info.frame_num` instead of counting packets.

### Open risks

| Risk | Status (2026-10-06) |
|---|---|
| BakkesMod records while RLBot runs the game | not yet observed; supported by the wiki's own `freeplay` option text and LAN guide (BakkesMod first, then launch through RLBot) |
| State setting from a bot is accepted by core | refuted as a risk by core's source: no bot/script distinction, only `enable_state_setting` |
| Packets arrive at 120 per second | not yet observed; see the packet-rate note above |
| Protocol mismatch between crate and core | refuted: schemas wire-identical |
| Roll sign of the start rotation | not yet observed |
| Run-to-run noise | not yet measured; run each scenario twice and diff |

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
