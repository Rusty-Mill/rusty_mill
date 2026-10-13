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
| `GameLink`, `GameCommand` | the `TcpMessenger` send methods RLBotCS calls, and `Input.cs` / `RunConsoleCommand.cs` |
| `SimGameLink` | a game that is `rb_env` (via `rb_rlbot_core::Simulation`) |

## The real game

`game_wire` encodes and decodes Rocket League's `-rlbot` socket payloads (format:
[`RL-GAME-SOCKET-FORMAT.md`](../../rusty_bullet/docs/research/RL-GAME-SOCKET-FORMAT.md), read
from `Bridge.dll` under ADR-0083). `GameFolder` turns the game's events into the `GamePacket`
bots get; `RocketLeagueLink` runs both over any stream and is a `GameLink`. `rb_rl_game_probe`
listens like RLBot does and prints (and with `--record` saves) what a live game sends;
`--analyze FILE` summarises a recording and `--commands FILE` prints what was sent to the game
(`rb_rl_bridge --record-in` / `--record-out` make those files); `--census-in FILE` /
`--census-out FILE` tally the message and command tags in such recordings and flag any outside
the known unions (`Census`). `rb_rl_demo_count FILE` reads a `--record-in` recording and lists, per
map, every player spawn, every destroy of a player actor and every stretch a car was reported
Demolished, so a demolition can be told from a destroy-and-respawn; it prints actor ids and teams,
never display names. `rusty_bullet/tools/rl_live_test` drives a whole live run with it.

Checked against committed buffers made with the DLL's generated codec. The author reports a
live Epic Freeplay run on 2026-10-11, totaling 14,993 and 2,524 inbound messages and a bot reaching
about 2300 uu/s. The committed slices reproduce 250 messages/513 physics frames (human) and
51 messages/115 frames (spawned bot), plus one accepted SpawnBot and one PlayerInput. They do
not reproduce the full-session totals or establish which exact build/clean-tree state produced
them; that build provenance is unavailable here. `tests/live_capture.rs` now uses strict decoding
for supported events. Tolerant decoding's zero errors alone does not establish that every known
event decoded successfully. No new live run or capture was performed for the review corrections.

Not yet tried: a timed match, several bots,
a Steam install. Not ported: forcing a loaded map to `Paused` (spawning does not wait for it), the spawn-command
queue's batching, rendering, the stat and Rumble/Dropshot events, boost interpolation between
`PlayerBoostChange`s, and human players being spawned (a human's car takes the next free index).

## Bots in the real game

`LinkBackend` is a `rb_rlbot_core::MatchBackend` over a game stream: it reads the game
without blocking, applies the distribution rules, and sends bot input and state setting back
(ball and car placement, boost). `rb_rl_bridge` puts it together:

```
rb_rl_bridge [--game-port N] [--port N] [--launch RocketLeague.exe] [--match-length 5|10|20|unlimited]
             [--no-spawn] [--record-in FILE] [--record-out FILE] AGENT_ID:TEAM...
```

Declare the attached match's duration with `--match-length` (default 5 minutes; use `unlimited`
for Freeplay). This option describes the game; it does not change its settings.

Start the game with the printed `-rlbot` arguments and a match loading, then start the bots with
`RLBOT_SERVER_PORT` set. Once the map has been up for two messages the bridge sends one `SpawnBot`
per `AGENT_ID:TEAM`, in that order, and the match starts when every car has answered (a bot's
handshake completes then). Each car is tied to its player index by the spawn command's id, as in
RLBot, so the game may answer in any order; indices stay put when a car leaves, using demolished placeholders for empty slots. Configured
player IDs match the handshake. Bot slots are reserved before any unsolicited car arrives, and
readiness requires a distributed snapshot containing the acknowledged mapping. `--no-spawn`
expects the cars to exist already. Bots get a standard Octane loadout in plain team colours:
**unverified in the game**, and the configuration's own loadouts are not modelled. Tested end to end with a stand-in game (`tests/real_core.rs`): a real
`rb_rlbot_client` bot sees the game's field and moving car, its input reaches the right actor, and
its state setting becomes a ball placement.

## Limits worth knowing

- **Start `rb_rl_bridge` before the game.** The game dials it once, at launch, and spawning waits
  for the map announcement that follows. A bridge that joins a game already in a match never sees
  that announcement, so it spawns nothing and the match never starts for it.
- **The game is the connection that speaks like it.** `GameListener::wait_for_game` takes a
  connection for the game only once its first message arrives and decodes as one of the game's
  (it looks without reading, so the link still sees that message). A connection that closes,
  sends something else, or stalls part-way through a message (`STALL_AFTER`, ten seconds: `peek`
  cannot see a close behind buffered bytes) is dropped and the wait goes on; a silent one waits,
  because the game says nothing until a map loads. At most `MAX_CANDIDATES` wait; a newer
  arrival drops the oldest, in arrival order, after every waiting connection has been looked at,
  so a game that has spoken is never the one dropped. Limits: this is not authentication (a
  local process that speaks the game's format first is taken for the game), and a game still
  silent can be dropped by more than `MAX_CANDIDATES` connections arriving after it; it dials
  once, so it would then be lost.
- **What the bridge prints, in order.** `serving bots on ADDR` as soon as the bots' port is
  taken (bots may start now); `a connection from ... is waiting to speak` when something dials
  the game port (the live-test runner waits for this one: the game has dialled); `the game
  connected` when a connection has spoken as the game (a map has loaded).
- **The bots' port is taken at startup**, before the game is launched: a clash fails at once
  naming the port, and a bot that starts while the game is still loading connects and waits for
  its handshake instead of being refused. (How long a given bot waits is the bot's own timeout.)
- **A spawn the game never answers is not retried** and has no timeout; the match then never
  starts. (Resending could create two cars.)
- **A socket that takes part of a frame** (a stalled game on a non-blocking connection) is
  handled: unsent bytes wait in an outbox, ahead of later frames, and are written as the link next
  runs, so the game never sees a half frame followed by a new one. Past 256 KiB waiting, sends are
  refused with `WouldBlock`.
- **One odd event does not end a session, unless it carries state.** An unreadable event
  (an enum value or required field this crate does not know, or no body) decodes as
  `GameEvent::Other(tag)` when losing it costs little: physics, boost and the clock are re-sent as
  they change, and hits and stats are not tracked. A spawn, a
  destroy, the match phase, a score or a pad's state is announced once and holds until the next event of its own kind, and the game has
  no known command to resend or snapshot it (the `Exec` console command is untested for this), so an unreadable one is a fault (`WireError::State`, naming
  the event) and the session ends (library callers must stop and drop the link after the `Err`;
  there is no graceful `DisconnectSignal` at that point); carrying on would leave the link wrong for the rest of the
  match (a lost `PlayerSpawn` means a spawn acknowledgement that never arrives). The only
  resynchronisation the protocol offers is a new map: its `GameSpawn` announces everything again.
  A message with more than 4096 events or more than 64 KiB of names is refused, since a crafted
  buffer can make many slots share one string.
- **Analysis is strict for supported events.** `decode_game_packet_strict` and the probe report
  missing required fields and invalid validated enums (`MatchPhase`, `PickupStateType`); other
  raw enum bytes are not range-validated, and unsupported event tags remain `Other`. Partial final frames are
  errors in analysis and live links. Permanently oversized commands return `InvalidInput`.
- **Recordings use new files only.** Existing paths and aliases are refused without truncation,
  preventing inbound and outbound recordings from sharing a file.
- **Recordings contain the players' display names** (`--record`, `--record-in`). Scrub them before
  sharing, as the fixtures here were.

## Differences from RLBotCS

- `tcp_port_is_free` tries `0.0.0.0` **and** `127.0.0.1`. RLBotCS binds only the wildcard, which
  on Windows succeeds while something holds `127.0.0.1:port`, exactly where `GameListener`
  listens. A test found this.
- `GameLaunch` starts the executable directly (as `Epic.Windows.cs:55-62` does); Steam, Epic,
  legendary and Heroic launchers are not driven.

## Verified

`cargo test -p rb_rl_game_bridge`: unit tests, the loop and listener, the golden codec, the real
link (fold, commands, byte budget, spawning), a stand-in game end to end, the census, the state-fault
robustness checks, and tests on recordings of a real game. Run live on 2026-10-11 (above).

## Starting the game in RLBot mode (Epic, Windows)

RLBot's own method, as a script: start the game through the Epic launcher so it writes its login
args to `Documents\My Games\Rocket League\TAGame\Logs\Launch.log`, read the path and args from
`Init: Base directory:` and `Init: Command line:`, close the game, and start `RocketLeague.exe`
with `-rlbot RLBot_ControllerURL=127.0.0.1:<port> RLBot_PacketSendRate=240 -nomovie` followed by those
login args (credentials: never print or log them). Then start a match (Freeplay is enough) and
`rb_rl_bridge` spawns and drives the bots.

Close the game as soon as a **new** `Init: Command line:` line appears, as RLBot's
`Epic.Windows.cs` does, not after it reaches the menu: a game that has logged in seems to have
spent the single-use login code, and the relaunch then sits at "click to start" (seen twice; see
[`RL-LIVE-TEST-2026-10-12.md`](../../rusty_bullet/docs/research/RL-LIVE-TEST-2026-10-12.md)).
`rusty_bullet/tools/rl_live_test` does the whole sequence.
