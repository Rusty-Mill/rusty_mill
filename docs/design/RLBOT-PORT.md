# RLBot application port: scope

Status: **scope written 2026-10-11; build order steps 1 to 3 are on main** (checked 2026-10-15, see
section 6 for what is and is not built). Complements
`crates/apps/rocket_league/rlbot/PLAN.md`, which covered only the bot-side client (stages 1 to 5, all
merged). This note covers the application itself: launcher, GUI, core server and match config.

## What was read, and what was not

Read for this note, from GitHub, at the commit the owner's earlier review pinned for RLBot core
(`1cc7b0c`, which reports itself as `RLBotServer v5.0.0-rc17`): `core` `README.md`, `Main.cs`,
`BotLauncher.cs`, `RocketLeagueLauncher.cs`, the Epic, Steam and custom launcher files, and parts of
`MatchStarter.cs`, `BridgeHandler.cs` and `ConfigParser.cs`; directory listings of the core tree; the
`flatbuffers-schema` repository's 11 `.fbs` files (from `main`, not from core's pinned submodule); the
`launcher` README; a summary of the `gui` repository page. **Not read:** `Bridge.dll` (closed source),
`FlatBuffersSession.cs` and the rest of the server, rendering, quick chat, the GUI and launcher sources,
the botpack. Anything below that depends on those is marked.

## Goal, as restated

Own the RLBot environment end to end, so that remote and local setups stop disagreeing. Running
existing RLBot v5 bots unmodified is a goal, not a gate. The scope is the application a person
runs, not a bot framework.

## 1. How the application is built

| Part | Language, licence | Role (source) |
|---|---|---|
| **Launcher** (`RLBot/launcher`) | Rust, MIT | Installs and updates `rlbotgui.exe` and `RLBotServer.exe` in `%LOCALAPPDATA%/RLBot5`, and updates itself (README) |
| **GUI** (`RLBot/gui`) | Go with Wails v3 and Svelte, MIT | Match setup by dragging bots onto teams, mutators and presets, starts matches, downloads verified bots from the botpack, loadout editor, sandbox (repository page summary; sources not read) |
| **Core** (`RLBot/core`, `RLBotServer`) | C# on .NET 10, MIT | The server. Two threads joined by channels: a TCP server for bots, scripts and managers on port 23234 (`FlatBuffersServer`), and a bridge handler that talks to the game (`BridgeHandler`) (`Main.cs`) |
| **Bridge** (`Bridge.dll`) | closed source | The game-facing protocol, "built from a closed-source repository due to legal reasons ... maintained by RLBot developers who have signed an agreement with Psyonix" (core README) |
| **Schema** (`RLBot/flatbuffers-schema`) | FlatBuffers; no licence file found | 11 `.fbs` files: 59 tables, 11 structs, 50 enums, 6 unions |
| **Ball prediction** | native Rust library `rl_ball_sym` (VirxEC), loaded by core | Predicts the ball for bots that ask (core README) |

Inside core (`RLBotCS`): `ManagerTools` holds `LaunchManager` (game and bot launching), `MatchStarter`
(match lifecycle), `ConfigParser` (about 940 lines, TOML via the Tomlyn library), `PlayerSpawner`,
`BallPredictor`, `Rendering`, `QuickChat`, `PerfMonitor`; `Server` holds the bot-facing server and the
game-facing bridge handler; `Conversion` turns game state into schema messages.

### How it talks to the game

Core launches Rocket League with `-rlbot RLBot_ControllerURL=127.0.0.1:<gamePort> RLBot_PacketSendRate=240
-nomovie` (`RocketLeagueLauncher.cs`). The game then connects **to core**, and `TcpMessenger` and
`MessageHandler` (both in the closed `Bridge`) decode what the game sends. Core tracks the match phase,
spawns the map and cars with console commands, distributes a packet to bots when game time advances, and
rate-limits rendering. Launching differs by platform (`LaunchManager`):

- **Steam**, Windows and Linux (Proton): start the game with those arguments.
- **Epic**, Windows: start the game normally, read its launch log for the path and login arguments, kill
  it, and relaunch with the RLBot arguments. This looks like a plausible source of "works here, not there" (an inference, not something the source states).
- **Epic**, Linux: `legendary launch Sugar ... -noeac`, or Heroic.
- **Custom**: a user-provided launcher.

### How bots and scripts are started

`BotLauncher.cs`: run `run_command` in a shell, working directory `root_dir`, environment from the config
plus `RLBOT_AGENT_ID` and `RLBOT_SERVER_PORT`; an empty `run_command` means "start it yourself"; a
non-zero exit code is logged. Scripts are the same. This is the whole contract a bot relies on.

## 2. What must be ported, and the legal line

| Component | Port? | Reasoning |
|---|---|---|
| Bot and script process lifecycle | Yes, small | The contract above. About 100 lines in C# |
| Match configuration (`match.toml`, `bot.toml`, `script.toml`, `loadout.toml` and the schema's `MatchConfiguration`) | Yes | `ConfigParser.cs` is the reference; needs a TOML reader (see section 4) |
| Bot-facing TCP server and handshake | Yes | Public protocol; our `rb_rlbot_wire` and the fake core in `rb_rlbot_client` tests already speak half of it |
| Match lifecycle, tick distribution, field info | Yes, against our simulator | The game side of `MatchStarter` and `BridgeHandler`, with `rb_env` in place of the game |
| Ball prediction | Optional | Our own simulator could replace `rl_ball_sym` |
| Launcher (installer, updater) and GUI | Later | Replaceable, but not on the critical path |
| **Game-facing bridge** | **No** | Closed source, under an agreement between RLBot and Psyonix. Reimplementing it means decoding the game's own API protocol, which is the binary and protocol reverse engineering that RB-RESEARCH-O002 found likely to breach the EULA. A native replacement of core against the real game would still need it |

This confirms reading A in `RESEARCH-BACKLOG.md` O007 and rules it out.

## 3. Options

| Option | What it is | Fixes env parity? | Risk |
|---|---|---|---|
| **A. Rust manager over the real core** | A Rust replacement for the GUI and launcher: reads TOML, starts RLBotServer and the game, sends the match configuration over the public protocol, manages bots. The core and bridge stay as they are | No. Still needs Windows, the game and the same Epic and Steam quirks, though the manager's own behaviour becomes ours | Low legal risk (public protocol); moderate effort |
| **B. Sim-backed core, no game** | A Rust server speaking the bot-facing protocol and stepping `rb_env` instead of Rocket League. Bots, scripts and managers connect unchanged. Runs headless on Linux, deterministically | **Yes**, for everything that does not need the real game: regression, scenario sweeps, training, CI | Fidelity: results are only as good as the simulator (see the parity table); not a way to play the real game |
| C. Native core against the real game | Port core including the game bridge | Would | **Blocked**: the bridge is closed and under agreement; legal exposure |

**Recommendation: B first, A later and separately, C not at all.** B addresses the stated pain, reuses the
most of what exists (the simulator, `rb_env` with its match flow, the wire crate and client, `rb_chaser_bot`),
has no legal exposure beyond what the project already carries, and is exactly what O007 recommended. A
remains available when the real game must be driven. This is the owner's decision.

## 4. What exists and what is missing in rusty_mill

| Need | Have | Missing |
|---|---|---|
| FlatBuffers runtime | `rusty_flatbuffers` (reader, builder) | verifier for hostile input if the server faces untrusted clients |
| Protocol messages | `rb_rlbot_wire`: client messages 7 of 17 (`MatchConfiguration`, `PlayerInput`, `DesiredGameState`, `ConnectionSettings`, `StopCommand`, `InitComplete`, `PingResponse`), core messages 6 of 10; 34 hand-written types against the schema's 126 definitions (59 tables, 11 structs, 50 enums, 6 unions), verified byte for byte against `rlbot_flat` 0.6.0 and on the real core | the rest, notably comms, ball prediction, rendering, console commands, loadouts, most mutators; a generator once the subset passes about 60 types (`PLAN.md`) |
| Bot-side client | `rb_rlbot_client`, `rb_rlbot_bridge`, `rb_chaser_bot` | nothing for this scope |
| Server side | an in-process fake core in the client's tests | a real server: accept, handshake, ping, readiness, distribute packets, apply `PlayerInput` and `DesiredGameState` |
| Simulator and match flow | `rb_physics_bullet`, `rb_env` (flow, boost pads, kickoff, chaser) | packet building from `Env` state (cars, ball, pads, match phase, scores), field info |
| Process confinement | `rusty_sandbox` (Linux, fail-closed: Landlock, seccomp, rlimits) | an adapter, optional |
| Web front end | `rusty_serve` (blocking HTTP server with a static UI); `rleval-app` carries its own small server, so there is a precedent for a web UI | the pages; which server to use |
| Native UI | `rusty_gpu`, `rusty_gui` exist; their status was not checked | decision |
| TOML | none in-house; `toml = "0.8"` is already a workspace dependency used by several app crates (`rusty_key`, `rusty_provider`, others) | decide between that dependency and a small purpose-built reader; the four config files use a small subset |

## 5. The wire protocol as a spec (feeds task 6)

Framing and layout, as implemented and verified against the real core: a `u16` big-endian length, then a
FlatBuffers `InterfacePacket` (client to core) or `CorePacket` (core to client), each a single required
union field. The unions at schema `main` are:

- **Client to core (17):** `DisconnectSignal`, `StartCommand`, `MatchConfiguration`, `PlayerInput`,
  `DesiredGameState`, `RenderGroup`, `RemoveRenderGroup`, `MatchComm`, `ConnectionSettings`, `StopCommand`,
  `SetLoadout`, `InitComplete`, `RenderingStatus`, `PingRequest`, `PingResponse`, `UpdatePerformanceMonitor`,
  `ConsoleCommand`.
- **Core to client (10):** `DisconnectSignal`, `GamePacket`, `FieldInfo`, `MatchConfiguration`, `MatchComm`,
  `BallPrediction`, `ControllableTeamInfo`, `RenderingStatus`, `PingRequest`, `PingResponse`.

Plan for the spec document: one table per message with fields, types, required or optional, and the
behaviour that is not in the schema (ordering and readiness: connect, `ConnectionSettings`, receive field
info and team info, `InitComplete`, then a `PlayerInput` per packet), with golden vectors from the real core
for each. Two cautions: the schema repository shows **no licence file**, so the spec must be written from the
protocol's behaviour and our own verified vectors, with the schema revision stamped, and the licence question
asked of the maintainers; and `rb_rlbot_wire` is pinned to schema revision `c38374e` while core's submodule
revision was not read, so version drift is a known gap.

## 6. Crate layout, build order and the first slice (option B)

Crates, under `crates/apps/rocket_league/crates/` (names are proposals; `rb_rlbot_*` per PLAN.md):

| Crate | Responsibility |
|---|---|
| `rb_rlbot_wire` (extend) | the remaining messages the server must read and write |
| `rb_rlbot_match` | the match configuration model and the TOML files |
| `rb_rlbot_launch` | bot and script process lifecycle: `run_command`, `root_dir`, environment, exit reporting; optional sandbox |
| `rb_rlbot_server` | bot-facing TCP server, handshake, readiness, ping, packet distribution, applying inputs and state |
| `rb_rlbot_core` (binary) | ties the server to `rb_env` at 120 Hz and builds `GamePacket` and `FieldInfo` from its state |

Build order, each its own PR:

1. Server-side messages in `rb_rlbot_wire`, with golden vectors. **Built** (#621).
2. `rb_rlbot_server` against the existing client tests (replace the fake core). **Built, but not as its own
   crate:** the socket server is the `Core` in `rb_rlbot_core`, so `rb_rlbot_server` does not exist.
3. `rb_rlbot_core`: `Env` to `GamePacket`; **first vertical slice** below. **Built**, with lockstep and
   real-time pacing and a binary; the slice's done-criteria are not yet tested (see below).
4. `rb_rlbot_launch` and `rb_rlbot_match`: start bots from a `bot.toml` and a `match.toml`. **Not started**;
   neither crate exists, so bots are started by hand.
5. Boost pads, scores and match phases in the packets; `DesiredGameState`. **Built**: `Simulation::packet`
   emits boost-pad states and timers, scores and match phases (`sim.rs`), with unit tests, and desired state
   is applied and bounds-checked. The one gap is match info (gravity, game speed), which `rb_env` cannot change
   and the core ignores. Not verified at runtime or against the game.
6. Stretch: stock v5 bots unmodified (a Python bot, a Rust bot, a script, a hivemind). **Not started.**
7. Later: option A's manager, launcher and GUI. **Not started.**

**First slice:** start `rb_rlbot_core`, which launches the existing `rb_chaser_bot` binary with
`RLBOT_SERVER_PORT` and `RLBOT_AGENT_ID` set, as core does; the bot connects, handshakes, receives a
`GamePacket` every tick, and sends a `PlayerInput`; the car drives at the ball in `rb_env`. Done when the
ball is touched within a fixed number of seconds and two runs produce identical frames. No new
dependency; the bot already exists and has run against the real core.

Status of the slice (2026-10-15): `rb_rlbot_core` runs with `rb_chaser_bot` (its README has the recipe), but
the bot is started by hand in a second shell, not launched by core. Repeatability is covered at component
level: `the_same_bot_gives_the_same_match_every_time` (`tests/core.rs`) compares two 900-packet runs through
the real socket `Core` and client with a fixed-input test bot. Not covered: a process-level test that the
`rb_chaser_bot` executable touches the ball within a fixed time, which the slice's done-criteria still need.

## 7. Risks

- **Fidelity.** Option B is only as faithful as the simulator; the parity pass found open pad-acquisition
  and caption-only rows. Results from B are for regression and training, not a claim about the real game.
- **Bot expectations.** Real bots may rely on behaviour the schema does not state (ball prediction, rendering
  acknowledgements, timing). The stretch goal measures this; it is not a gate.
- **Packet rate.** The game is asked for 240 packets per second (`RLBot_PacketSendRate=240`) and core
  distributes one per advance of game time; our earlier real-game probe saw about 120 per second. The server
  must decide what rate it offers.
- **Schema licence and drift** (section 5).
- **Platform and terms.** The real game side is Windows-heavy (Epic login harvesting) and offline only under
  Easy Anti-Cheat; option B avoids both, option A inherits them. Nothing here changes the EULA position in
  `docs/research/RL-SERVER.md`.
- **Upstream churn.** The v5 server is pre-release (`rc17`); anything modelled on it can move.

## 8. Open questions

1. Confirm the recommendation: B first, A later, C dropped.
2. Is "task 6" the wire spec? This note assumes so.
3. TOML: reuse the workspace's `toml` dependency, or write a small reader? The sovereignty policy suggests the
   latter; the cost is small for four simple file types.
4. Ask RLBot about the schema's licence before the spec is published.
5. Should option A's GUI be a web UI on `rusty_serve` (as `rleval-app`) or native?
6. What does the real core do on aspects not read here (`FlatBuffersSession`, readiness, comms, rendering)?
   Read it before step 2 of the build order.

## PR #623 scope clarification

The exclusion above describes the original plan. ADR-0083 in
`crates/apps/rocket_league/rusty_bullet/docs/adr/` records the owner's narrow exception for
extracting the game-socket wire format from Bridge.dll, confirmed in PR #623. That PR adds
the game-facing codec/link as well as the simulated core. It does not authorize copying DLL
logic, expand the exception to game code/assets, or establish legal clearance.
