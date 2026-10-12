# rb_rlbot_core

A simulated RLBot v5 core: bots and scripts connect to it exactly as they connect to RLBot core
(port 23234, `rb_rlbot_wire` framing), but the match is played on `rb_env`'s physics, so no
Rocket League is needed and a run can be made repeatable. How the real core reaches the game
is in [`RLBOT-BRIDGE-DLL.md`](../../rusty_bullet/docs/research/RLBOT-BRIDGE-DLL.md).

| | |
|---|---|
| `Simulation` | The match with no sockets: `new(MatchConfiguration)`, `set_input`, `step`, `apply_desired`, `packet`, `field_info`, `controllable_team_info`. Deterministic. |
| `Core` | The socket server around it: `bind`, `run(&stop, on_event)`. Single thread, `std::net`, no async. |
| `MatchBackend` | What `Core` plays on. `Simulation` is one (stepped by core at its pacing); `rb_rl_game_bridge::LinkBackend` is a real game, which paces itself, so core polls it from the start and holds a bot's opening reply until the game has announced its map. |
| `Pacing::Lockstep` | A tick runs only after every controlling bot has answered the last packet (or a timeout). Same bots, same match, whatever their speed. |
| `Pacing::RealTime` | One tick per 1/120 s with each car's latest input, as the real game feels to a bot. |
| `rb_rlbot_core` binary | `rb_rlbot_core [--port N] [--lockstep] [--length five\|ten\|twenty\|unlimited] AGENT_ID:TEAM...` |

## Run a bot against rusty_bullet

```
cargo build --release -p rb_rlbot_core -p rb_rlbot_bridge --bins
target/release/rb_rlbot_core --lockstep rusty_bullet/chaser_bot:0
# in another shell
RLBOT_SERVER_PORT=23234 RLBOT_AGENT_ID=rusty_bullet/chaser_bot target/release/rb_chaser_bot
```

The match starts when every named bot has connected and sent `InitComplete`; a stop command from
a client ends the run. Core prints each `Event` (joins, readiness, rejected input, dropped
clients) because it logs nothing itself.

## Behaviour

- **Handshake.** `ConnectionSettings` with an agent id gets `ControllableTeamInfo` (the cars
  configured with that id), `MatchConfiguration` and `FieldInfo`; the bot then sends
  `InitComplete`. An empty id (a logger or match runner) gets the last two and packets at once,
  and may not send input.
- **Input** is accepted only for a car the sender controls (`Event::InputRejected` otherwise) and
  is held until replaced. The first kickoff is `rb_env`'s 850-tick countdown, during which
  input is ignored, as in the game.
- **State setting** (`DesiredGameState`) changes the first ball and the listed cars: only the
  fields named. Boost is clamped to 0-100. Honoured only if the configuration enables state
  setting.
- **Bad clients are dropped, not fatal:** an undecodable frame, a second connection with a taken
  agent id, a socket that stops reading (2 s write timeout).

## Not modelled

Ball prediction, match comms, rendering, quick chat, loadouts, console commands, starting a new
match over the socket, gravity and game speed in state setting, and every mutator except match
length. `air_state` is only on-ground or in-air (from the car's height); `demolished_timeout` is
always -1. Anything a client asks for beyond this is ignored, never a crash.

## Verified

`cargo test -p rb_rlbot_core`: 8 simulation tests; 6 socket tests that drive the core with the real
`rb_rlbot_client` over loopback, including two full runs compared packet for packet. The built
`rb_chaser_bot` process joined the built `rb_rlbot_core` process and the match started (hand run,
2026-10-11). Not verified: against RLBot's own Python or C# clients, or the RLBot GUI.
