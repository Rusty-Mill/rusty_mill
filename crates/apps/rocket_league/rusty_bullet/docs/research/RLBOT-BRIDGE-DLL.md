# How `Bridge.dll` connects RLBot and Rocket League

Status: historical open-source research, 2026-10-11, followed by the wire-format work below.
The initial sections describe RLBot core's open source (`RLBot/core`, v5.0.0-rc.17,
author checkout `C:\dev\core`). Subsequently, ADR-0083 authorized reading `Bridge.dll`
for its wire format only; owner approval is recorded on PR #623. The original statement
that the DLL had not been opened applies only to the initial research. It is not a claim
about the later codec work or legal clearance. Paths below are relative to `RLBot/core`.

## What it is

| Claim | Evidence |
|---|---|
| `Bridge.dll` is a managed .NET assembly that RLBotCS references, not native code it P/Invokes. | `RLBotCS/RLBotCS.csproj:98-100`: `<Reference Include="Bridge"><HintPath>lib\Bridge.dll</HintPath>`. Plain `using Bridge.*;` throughout (`Server/BridgeHandler.cs:3-4`, `Server/BridgeContext.cs:2-4`). |
| It is closed-source. | `README.md:46-48`: "The `Bridge.dll` file in `RLBotCS/lib` is built from a _closed-source_ repository due to legal reasons." |
| It is versioned separately and patched in place. | `Main.cs:14` prints `Bridge {BridgeVersion.Version}`; git history of `RLBotCS/lib/Bridge.dll`: `8cf6379 Patch bridge to v0.9.2`, `8b35470 Patch Bridge.dll`. |
| It owns the game-facing types. | Namespaces used from it: `Bridge.TCP` (`TcpMessenger`), `Bridge.State` (`GameState`), `Bridge.Conversion` (`MessageHandler`), `Bridge.Controller`, `Bridge.Packet`, and `Bridge.Models.{Message,Phys,Command,Control,Render}` (`Conversion/FlatToModel.cs:1-11`, `Conversion/GameStateToFlat.cs:1-11`). |

## The two sockets

RLBot core is a middleman with a socket on each side. Only the bot side is open.

```
 bots / scripts                RLBotCS                     Rocket League
 (rb_rlbot_client)      (this is what Bridge.dll serves)    (RocketLeague.exe)

   :23234  <---FlatBuffers--->  FlatBuffersServer
                                     |  channels
                                BridgeHandler  <---Bridge.dll TcpMessenger---  :23233
                                                    (game dials OUT to us)
```

1. **Bot side, port 23234.** `Main.cs:82` creates `FlatBuffersServer(rlbotSocketsPort, ...)`, which
   listens on `IPAddress.IPv6Any` (`Server/FlatBuffersServer.cs:19`). Framing is the `u16` length
   plus FlatBuffers payload already implemented in `rb_rlbot_wire`. Default port
   `RlbotSocketsPort = 23234` (`ManagerTools/LaunchManager/RocketLeagueLauncher.cs:8`). Launched
   bots are told where it is through `RLBOT_SERVER_PORT` and `RLBOT_AGENT_ID`
   (`ManagerTools/LaunchManager/BotLauncher.cs:27-29`).
2. **Game side, port 23233 (default).** `Main.cs:97` creates `new TcpMessenger(gamePort)`, a
   `Bridge.TCP` type, and hands it to `BridgeHandler`. **The game connects to RLBot, not the
   other way round**: RLBot starts Rocket League with

   ```
   -rlbot RLBot_ControllerURL=127.0.0.1:<gamePort> RLBot_PacketSendRate=240 -nomovie
   ```

   (`RocketLeagueLauncher.cs:14-20`, `GetRLBotArgs`; used by Steam, Epic, legendary and Heroic
   launchers alike: `CustomLaunchers.cs:9,16`, `Epic.Windows.cs:50`, `Steam.Linux.cs:400`).
   `TcpMessenger.WaitForConnectionAsync()` (`BridgeHandler.cs:56`) is the accept. These are
   Rocket League's own `-rlbot` hooks; Psyonix built the game side.

The wiki analysis already in this repo agrees (`RLBOT-V5-WIKI-ANALYSIS.md:82`): "Rocket League
itself exposes a controller URL on 23233 and a packet send rate (240 here vs the 120 Hz
physics tick) via command-line args; RLBotServer (23234) sits on top of that."

### Picking and finding the game port

`ManagerTools/LaunchManager/ProcessUtils.cs`:

- `FindUsableGamePort` (86-113): if a `RocketLeague` process is already running with a
  `RLBot_ControllerURL` argument, reuse its port; otherwise try ports from `IdealGamePort = 23233`
  up, skipping the sockets port, by binding a `TcpListener` on `IPAddress.Any`; fall back to
  `DefaultGamePort = 50000` (`RocketLeagueLauncher.cs:9-10`).
- `IsRocketLeagueRunningWithArgs` (170-182): a `RocketLeague*` process whose command line contains
  `rlbot`. On Windows the command line comes from a helper (`ProcArgs.Retrieve`,
  `ProcArgs.Windows.cs`); on Linux from `/proc/<pid>/cmdline` (`GetProcessArgs`, 121-146).

## What `BridgeHandler` does with the game's messages

`Server/BridgeHandler.cs` is the loop that uses `TcpMessenger`. The methods of `TcpMessenger`
RLBotCS calls are exactly five: `WaitForConnectionAsync`, `ReadAllAsync`, `ResetByteCount`,
`WaitForAnyMessageAsync`, `Dispose` (a grep for `[Mm]essenger.` across the repo). Everything
else goes *into* the messenger through helpers that wrap it: `MatchCommandQueue`,
`SpawnCommandQueue`, `PlayerInputSender`, `Rendering` (`Server/BridgeContext.cs:40-43`).

Per message ("clump") from the game, `HandleServer` (`BridgeHandler.cs:63-187`):

1. Resets the per-frame outbound byte counter and render-clear counter (76-82). Every game
   message means at least one frame rendered, so this is the cap on bytes RLBot may send per
   frame.
2. Folds the clump into the held `GameState` via `MessageHandler.CreateUpdatedState` (85-88).
   Clumps are partial updates; `Bridge.dll` owns that merge.
3. Decides whether **game time advanced** by more than 1 ms (90-95). The game sends up to 240
   messages a second against 120 physics ticks, so about half do not advance time.
4. `ConsiderDistributingPacket` (229-267): sends a `GamePacket` to the bots only if time
   advanced, or, in `Replay`/`Paused`/`Ended`/`Inactive`, after more than `MAX_TICK_SKIP = 1`
   messages without progress; and only when every required car is present (all players if the
   match is inactive or ended, otherwise just the bots, "bots are always the lower indexes").
5. On a map-load message (117-144): reset counters, force the phase to `Paused` (a LAN match
   never reports it), clear renders, spawn the cars, announce the field, and skip the rest of
   the tick.
6. Releases queued spawn commands once the map has spawned, two messages have passed and the
   phase is `Paused` (175-184).

Outbound, from the bots to the game, `Server/BridgeMessage/*.cs` are the commands:
`Input` (player input, mapped from player index to the game's actor id, `Input.cs`),
`RunConsoleCommand`, `SetGameState`, `AddRenders`/`ClearRenders`, `StartMatch`, spawn and loadout
messages, `ShowQuickChat`. Their payload types are `Bridge.Models.Command.*`,
`Bridge.Models.Control.PlayerInput` and friends; how they become bytes is inside the DLL.

## What is and is not known

| | Known from open source | Only inside `Bridge.dll` |
|---|---|---|
| Launch arguments, ports, port discovery | yes | |
| Who connects to whom (game dials RLBot) | yes | |
| Tick, time-advance and distribution rules | yes | |
| Message *kinds* (state in; input, console, render, spawn out) | yes | |
| Byte format of the game socket (framing, encodings, message ids) | no | yes; now recovered, see below |
| How clumps merge into `GameState` | no | yes |

## What rusty_bullet builds from this

| Piece | Crate | Based on |
|---|---|---|
| Launch args, `GameLaunch`, port finder, process probe, `GameListener` | `rb_rl_game_bridge` | `RocketLeagueLauncher.cs`, `ProcessUtils.cs`, `TcpMessenger` accept |
| `TickTracker`, `should_distribute`, `BridgeLoop` | `rb_rl_game_bridge` | `BridgeHandler.cs:63-187, 229-267` |
| `GameLink` trait and `SimGameLink` (the game is `rb_env`) | `rb_rl_game_bridge` | the five `TcpMessenger` methods |
| A whole simulated core: bots connect on 23234 and play on `rb_env` | `rb_rlbot_core` | `FlatBuffersServer` and the handshake the client already implements |

The simulated core is what makes runs **repeatable**: `Pacing::Lockstep` advances the match only
after every controlling bot has answered the previous packet, so the same bots produce the same
match whatever their speed (`rb_rlbot_core/tests/core.rs`,
`the_same_bot_gives_the_same_match_every_time`).

## The real game's byte format (done 2026-10-11, ADR-0083)

The owner approved reading `Bridge.dll` for the wire format only (ADR-0083). A decompiler run
in a scratch folder outside this repo showed that the game socket is **`u16` big-endian
length-prefixed FlatBuffers**, the same framing as the bot socket, with Psyonix's `psyonix`
schema inside. The table layouts are in [`RL-GAME-SOCKET-FORMAT.md`](./RL-GAME-SOCKET-FORMAT.md);
`rb_rl_game_bridge` implements them (`game_wire`, `RocketLeagueLink`) and was checked both ways
against the DLL's own generated code. Nothing decompiled is committed.

`rb_rlbot_core::Core` is now generic over a `MatchBackend`: the simulation is one, and
`rb_rl_game_bridge::LinkBackend` (a game behind `RocketLeagueLink` and `BridgeLoop`) is the
other, so bots play in the real game through the same server. `rb_rl_bridge` is the binary.

The bridge spawns the bots' cars itself (`SpawnBot`, tied to player indices by command id).

Run against a live Rocket League on 2026-10-11: it decodes, accepts `SpawnBot` and `PlayerInput`,
and the spawned bot drove (see the verification section of the format note, which also lists
what the game showed that the DLL's code did not).

Still open: a timed match, several bots, a Steam install, the colour swatches and configured
loadouts for spawned bots, and rendering.
