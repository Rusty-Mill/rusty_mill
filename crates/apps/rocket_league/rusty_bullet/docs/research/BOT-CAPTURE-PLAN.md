# Scripted-bot capture plan

Plan for `RB-RESEARCH-O005`: use RLBot to run repeatable, scripted mechanics
in the real game while the existing BakkesMod plugin records them, so each
mechanic gets a clean ground-truth capture with known inputs. Status: plan
only; nothing built. Sources: [RLBot FAQ](https://rlbot.org/faq/),
[RLBot v5 scripts](https://wiki.rlbot.org/v5/botmaking/scripts/),
[RLBot v5 index](https://wiki.rlbot.org/v5/),
[RLBotTraining](https://pypi.org/project/rlbottraining),
[state setting (v4 wiki)](https://github.com/RLBot/RLBotPythonExample/wiki/Manipulating-Game-State).

## What the RLBot repositories show (read 2026-10-05)

Sources: [core](https://github.com/RLBot/core),
[rust-interface](https://github.com/RLBot/rust-interface),
[gui](https://github.com/RLBot/gui),
[botpack](https://github.com/RLBot/botpack),
[wiki](https://github.com/RLBot/wiki),
[RLBot](https://github.com/RLBot/RLBot) (the old v4 repo).

- **v5 is the current framework; the `RLBot/RLBot` repo is v4**, and says
  it is missing the source behind `RLBot.exe` for legal reasons. Use v5.
- **core** (C#, .NET 10) is the v5 server. It talks to the game, bots and
  scripts over FlatBuffers sockets, and builds on Windows and Linux. The
  game itself still needs to be installed; whether the game runs on Linux
  for this is not established.
- **gui** (Go, Wails, Svelte) launches matches, downloads the botpack, and
  has a **state-setting sandbox** (drag cars on the field, replay
  scenarios). Windows and Linux builds.
- **botpack**: community bots capped at about Nexto strength; no list of
  scripted mechanic bots found. Licence MIT unless a bot says otherwise.
- **rust-interface** (crate `rlbot`, MIT) is a Rust client for the v5
  socket API, which fits this repo's language. Its `high_jump_script`
  example (verbatim read) shows everything the tape player needs: a
  `ScriptAgent` with `tick(game_packet, packet_queue)` that pushes a
  `DesiredGameState` of `DesiredCarState { physics: Some(DesiredPhysics {
  velocity: ... }) }`, reading `player.last_input.jump` and
  `player.physics.velocity` from the `GamePacket`. A bot example,
  `atba_agent`, returns controller inputs from the same loop shape. I did
  not read the controller struct's field names; they come from `cargo doc`.
  Other examples: `atba_hivemind`, `atba_raw`, `packet_logger`,
  `start_match`, `stop_match`.

This makes the plan concrete: write the tape player and the state-setting
script in Rust with the `rlbot` crate.

## Why scripted, not skilled

For physics validation the bot needs to replay the same inputs from the
same start, not to play well. A scripted controller gives identical runs
(so run-to-run noise is measurable) and exact inputs; the community ML bots
(Necto, Nexto) play full matches and cannot be told to perform a named
mechanic.

## Pieces

1. **Scenario**: a small file naming the initial state (car position,
   velocity, rotation, spin, boost; ball state) and a per-tick input
   timeline `{tick, throttle, steer, pitch, yaw, roll, jump, boost,
   handbrake}`. Same fields as the `input` object in the capture format
   (ADR-0005). Plain JSON; one file per mechanic.
2. **RLBot script** (v5 scripts run beside a match, declared in a
   `script.toml`) sets the initial state with a `DesiredGameState` when the
   scenario starts, as in the `high_jump_script` example.
3. **RLBot bot** replays the input timeline tick by tick: a deliberately
   dumb "tape player".
4. **BakkesMod plugin** (`bakkesmod-plugin/rusty_bullet_capture`, already
   built) records the session as JSON Lines. It hooks the game's own
   vehicle-input event, so it records the inputs the game actually applied.
5. **Scoring**: `rb-verify --self-onestep` and `--self-kstep` on the
   capture, as for the owner's captures.

## Risks to settle first (one spike each)

All settled on 2026-10-06, first real session (`BOT-RUN-SHEET.md` session
1, `tools/rb_tape_bot/README.md`, ADR-0059):

- BakkesMod loads and records when core starts the game with `-rlbot`:
  yes. BakkesMod runs first, core relaunches the game, the injector
  attaches, `plugin load rusty_bullet_capture` works. Eleven captures.
- RLBot v5 runs on the owner's Epic install: yes (core rc17, launcher
  Epic).
- State setting lands on the expected tick and the first captured frame
  equals the set state: yes, to the decimal, including rotation (roll
  sign right), velocity, angular velocity, boost and the ball. The
  capture misses 2 to 7 ticks at the set frame itself.
- Core accepts state setting from a bot: yes (gated only by the match's
  `enable_state_setting`).
- The `rlbot` crate speaks core's protocol: yes, 0.6.0 against rc17.
- Packets at 120 per second: yes, 120 packets and 120 physics frames per
  second measured; 240 identical packets per second only while paused.
- Determinism: two `prompt_dodge` runs differ by 2.8 uu mean, 13.8 uu max,
  a one-tick offset of the jump; that is the noise floor.
- Terms: offline only, freeplay via RLBot's own `freeplay` match option.

Surprises: core reports `MatchPhase::Paused` for all of freeplay (the bot
now gates on the frame counter); the game pauses when unfocused; the
plugin records all-zero inputs for an RLBot-driven car
(RB-RESEARCH-O008); one 5-tick hole per capture (RB-RESEARCH-O009).

## Mechanics, in order

| # | Scenario | Answers |
|---|---|---|
| 1 | Jump, wait 1.5 s, dodge; and drive off an edge, wait, dodge | the 1.25 s second-jump window and the never-jumped case (FR-136) |
| 2 | Wavedash: jump, pitch up, land and flip within a few ticks | landing with a flip, tire grip, wheel pushback |
| 3 | Pogo: drop the car nose-first from height, jump on contact, repeat | hard landings (the 18.35 s residual), jump re-arm on contact |
| 4 | Speed flip: kickoff diagonal flip, cancel, boost | flip torque, air roll, boost |
| 5 | Half flip, stall, flip cancel | flip clock and cancel rule |
| 6 | Hard landings and wall or corner slides from set velocities | the 9.125 s and 18.350 s residuals |
| 7 | Ball under the wheels (state-set ball) and flip reset | wheel-ball contact, jump refill (77.667 s) |
| 8 | Ball-contact mechanics: dribble, flick | car-ball friction and hit impulse |

Rows 1 to 3 and 6 target open questions already recorded in
`docs/PROJECT-STATUS.md`.

## Not doing

- ML policies for any of the above; revisit only for mechanics that need
  decisions (see `RB-RESEARCH-O005`).
- A reusable harness. Same stance as ADR-0005: a one-off script per spike
  until a second use shows it is needed.

## Where the code would live

A standalone Cargo package outside the workspace (for example
`tools/rb_tape_bot`), so the `rlbot` dependency and its transitive crates
stay out of the workspace build, CI and `rb_domain`. Dependency
justification for the PR: it is the only maintained Rust client for the
v5 socket API, MIT licensed; hand-rolling a FlatBuffers client is worse.
It reads the scenario JSON and has no dependency on the physics crates.

## Built (2026-10-05, not yet run against the game)

`tools/rb_tape_bot` (standalone package, `rlbot` 0.6.0): scenario reader
with tests (`src/lib.rs`), a `BotAgent` tape player (`src/main.rs`) that
sets the start state with a `DesiredGameState` on its first packet and then
sends one `PlayerInput` per packet, `bot.toml`, and two scenarios
(`prompt_dodge.json` control, `late_dodge.json` for the 1.25 s rule). One
bot does both jobs, so no script/bot timing sync is needed; whether core
accepts state setting from a bot is an open risk. Run steps and scenario
format are in `tools/rb_tape_bot/README.md`.

Scenarios (11, 2026-10-05): prompt and late dodge, three wavedash timings,
speed flip, half flip, pogo, and reproductions of the `test2.jsonl` 18.308 s
landing and 8.95 s corner slide and the `hitjump.jsonl` 77.642 s car over
the ball. Each has a port prediction in `tools/rb_tape_bot/README.md`, reproducible with
`rb-verify --scenario tools/rb_tape_bot/scenarios/<name>.json` (`rb_scenario`
crate, ADR-0056).

The comparison step exists: `rb-verify --scenario <file> --against <capture>`
(FR-011, ADR-0057).

Two tooling options (2026-10-06, FR-012 and FR-013, ADR-0058):
`rb-verify --scenario-from <capture> <from-secs> <to-secs> [name]` cuts a
scenario from a window of a human recording (state at the first frame,
recorded inputs as the tape) so the bot can replay a real performance, and
`rb-verify --seed-first-frame` (first argument) lets the `--self*` modes
score a recording that starts airborne.

## Next

Done 2026-10-06: all eleven scenarios captured and scored (results in
`BOT-RUN-SHEET.md` and the tape-bot README). Predictions held for the
dodges, half flip and pogo; speed flip, the hard landing, the corner slide
and the car over the ball diverge by 100 to 250 uu and are the next
targets. Then a second run per scenario for its noise floor, and Stage 3
(human windows via `--scenario-from`).
