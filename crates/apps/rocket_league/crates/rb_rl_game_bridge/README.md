# rb_rl_game_bridge

The game-facing half of RLBot core, as far as RLBot's open source shows it. RLBot talks to bots
on one socket and to Rocket League through the closed-source `Bridge.dll` on another; this
crate rebuilds what the open code reveals of the second, with citations, in
[`RLBOT-BRIDGE-DLL.md`](../../rusty_bullet/docs/research/RLBOT-BRIDGE-DLL.md).

| | RLBotCS source it follows |
|---|---|
| `rlbot_args(port)`, `GameLaunch` | `RocketLeagueLauncher.cs:14-20`: `-rlbot RLBot_ControllerURL=127.0.0.1:<port> RLBot_PacketSendRate=240 -nomovie` |
| `find_usable_game_port`, `controller_port`, `any_started_with_rlbot`, `tcp_port_is_free` | `ProcessUtils.cs:86-113`, `170-182` |
| `ProcessSource` / `SystemProcesses` | `ProcessUtils.cs` `GetProcessArgs` (PowerShell on Windows, `/proc` on Linux) |
| `GameListener` | `TcpMessenger.WaitForConnectionAsync` (the game dials out to RLBot) |
| `TickTracker`, `should_distribute`, `Roster` | `BridgeHandler.cs:90-110, 229-267` |
| `BridgeLoop` | `BridgeHandler.HandleServer` |
| `GameLink`, `GameCommand` | the five `TcpMessenger` methods RLBotCS calls, and `Input.cs` / `RunConsoleCommand.cs` |
| `SimGameLink` | a game that is `rb_env` (via `rb_rlbot_core::Simulation`) |

## The real game

`game_wire` encodes and decodes Rocket League's `-rlbot` socket payloads (format:
[`RL-GAME-SOCKET-FORMAT.md`](../../rusty_bullet/docs/research/RL-GAME-SOCKET-FORMAT.md), read
from `Bridge.dll` under ADR-0083). `GameFolder` turns the game's events into the `GamePacket`
bots get; `RocketLeagueLink` runs both over any stream and is a `GameLink`. `rb_rl_game_probe`
listens like RLBot does and prints (and with `--record` saves) what a live game sends;
`--analyze FILE` summarises a recording and `--commands FILE` prints what was sent to the game
(`rb_rl_bridge --record-in` / `--record-out` make those files).

Checked against buffers built by the DLL's own generated code, both ways, and **against a live
Rocket League** (2026-10-11, Epic install, Freeplay): the codec decoded every message, the game
accepted `SpawnBot` and `PlayerInput`, and a spawned `rb_chaser_bot` drove. The recordings are in
`tests/fixtures/` and pinned by `tests/live_capture.rs`. Not yet tried: a timed match, several bots,
a Steam install. Not ported: forcing a loaded map to `Paused` (spawning does not wait for it), the spawn-command
queue's batching, rendering, the stat and Rumble/Dropshot events, boost interpolation between
`PlayerBoostChange`s, and human players being spawned (a human's car takes the next free index).

## Bots in the real game

`LinkBackend` is a `rb_rlbot_core::MatchBackend` over a game stream: it reads the game
without blocking, applies the distribution rules, and sends bot input and state setting back
(ball and car placement, boost). `rb_rl_bridge` puts it together:

```
rb_rl_bridge [--game-port N] [--port N] [--launch RocketLeague.exe] AGENT_ID:TEAM...
```

Start the game with the printed `-rlbot` arguments and a match loading, then start the bots with
`RLBOT_SERVER_PORT` set. Once the map has been up for two messages the bridge sends one `SpawnBot`
per `AGENT_ID:TEAM`, in that order, and the match starts when every car has answered (a bot's
handshake completes then). Each car is tied to its player index by the spawn command's id, as in
RLBot, so the game may answer in any order; indices stay put when a car leaves. `--no-spawn`
expects the cars to exist already. Bots get a standard Octane loadout in plain team colours:
**unverified in the game**, and the configuration's own loadouts are not modelled. Tested end to end with a stand-in game (`tests/real_core.rs`): a real
`rb_rlbot_client` bot sees the game's field and moving car, its input reaches the right actor, and
its state setting becomes a ball placement.

## Differences from RLBotCS

- `tcp_port_is_free` tries `0.0.0.0` **and** `127.0.0.1`. RLBotCS binds only the wildcard, which
  on Windows succeeds while something holds `127.0.0.1:port`, exactly where `GameListener`
  listens. A test found this.
- `GameLaunch` starts the executable directly (as `Epic.Windows.cs:55-62` does); Steam, Epic,
  legendary and Heroic launchers are not driven.

## Verified

`cargo test -p rb_rl_game_bridge`: unit tests, the loop and listener, the golden codec, the real
link (fold, commands, byte budget, spawning), a stand-in game end to end, and 11 tests on recordings
of a real game. Run live on 2026-10-11 (above).

## Starting the game in RLBot mode (Epic, Windows)

RLBot's own method, as a script: start the game through the Epic launcher so it writes its login
args to `Documents\My Games\Rocket League\TAGame\Logs\Launch.log`, read the path and args from
`Init: Base directory:` and `Init: Command line:`, close the game, and start `RocketLeague.exe`
with `-rlbot RLBot_ControllerURL=127.0.0.1:<port> RLBot_PacketSendRate=240 -nomovie` followed by those
login args (credentials: never print or log them). Then start a match (Freeplay is enough) and
`rb_rl_bridge` spawns and drives the bots.
