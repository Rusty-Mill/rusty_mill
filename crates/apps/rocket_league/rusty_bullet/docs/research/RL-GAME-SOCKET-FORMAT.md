# Rocket League's `-rlbot` game socket: wire format

Status: 2026-10-11, under ADR-0083. Recovered by reading `Bridge.dll` v0.9.2 (the copy in
`RLBot/core` at `5.0.0-rc.17`) with a decompiler run in a scratch folder outside this repo.
Nothing from the decompiled output is committed; this note and the Rust built from it describe
the **format** only. RLBotCS's open code supplies the one conversion marked (open). Not yet
checked against a live game; see "Verification".

The schema is Psyonix's, namespace `psyonix`, built with FlatBuffers 25.2.10.

## Transport

- RLBot listens on `127.0.0.1:<port>`; Rocket League connects out (`-rlbot
  RLBot_ControllerURL=127.0.0.1:<port>`). One connection. `TCP_NODELAY` on.
- Each message is a **`u16` big-endian length, then that many bytes** of one FlatBuffers
  buffer, in both directions. Same framing as the bot socket (`rb_rlbot_wire::frame`).
- Largest message 65535 bytes. RLBot also sends at most 65535 bytes **per rendered frame** in
  total and drops what is over, resetting the count whenever any message arrives from the game.

## Directions and roots

| Direction | Root table | Holds |
|---|---|---|
| game to RLBot | `GamePacket` | `app_seconds` f32 (slot 0), `messages` vector of `GameMessageWrapper` (1), `delta_physics_frames` u8 (2) |
| RLBot to game | `ControllerPacket` | `messages` vector of `ControllerMessage` (0) |

`GameMessageWrapper`: slot 0 the union tag (`GameMessage`, u8), slot 1 the union table.
`ControllerMessage`: slot 0 `command_id` (struct, u16), slot 1 the tag (`ControllerCommand`,
u8), slot 2 the union table. RLBot numbers commands 1, 2, 3 ... per queue (wrapping at 65535).

## Inline structs (little-endian, packed)

| Struct | Layout |
|---|---|
| `ActorID`, `CommandID` | `u16` |
| `Vector3` | `f32 x, y, z` (12 bytes) |
| `Rotator` | `f32 pitch, yaw, roll` (radians, 12 bytes) |
| `PhysicsState` | location `Vector3` @0, rotation `Rotator` @12, velocity `Vector3` @24, angular velocity `Vector3` @36 (48 bytes) |
| `Color` | `u8 r, g, b, a` |

## Enums (all `u8`, numbered from 0 in the order listed)

- `GameMessage` (union tag): NONE, GameSpawn, GameStateChange, GameTimeChange, GoalSpawn,
  PickupSpawn, PickupStateChange, PlayerSpawn, PlayerPhysicsChange, PlayerBoostChange,
  BallSpawn, BallPhysicsChange, BallHit, TeamScoreChange, ActorDestroyed, PlayerStatChange,
  PlayerStatEvent, PlayerSpectate, PlayerInputChange, PlatformSpawn, PlatformDamageStateChange,
  BallExtraDataChange, PlayerPickupChange.
- `ControllerCommand` (union tag): NONE, SpawnBot, DestroyActor, PlayerInput, RenderLine3D,
  RenderText2D, RenderText3D, RemoveRenderItem, Exec, SetPhysics, SetBoost,
  RenderLineSeries3D, EndMatch, SetPaused.
- `GameStateType`: Inactive, Countdown, Kickoff, Active, GoalScored, Replay, Paused, Ended.
  (The same order as RLBot's `MatchPhase`.)
- `TeamType`: Blue, Orange. `GameMode`: Unknown, Soccar, Hoops, Snowday, Rumble, Dropshot.
- `PickupType`: BoostPartial, BoostFull. `PickupStateType`: Idle, PickedUp.
- `PlayerPhysicsState`: OnGround, Jumped, DoubleJumped, Dodged, InAir, Demolished.
- `Shape`: NONE, SphereShape, CylinderShape, BoxShape.

## Game to RLBot messages (table slots; "S" = struct, "T" = table, "U" = union)

| Message | Slots |
|---|---|
| `GameSpawn` | 0 map_name string, 1 mode, 2 team_size u8, 3 field (T `FieldSetup`: 0 center S, 1 width, 2 length, 3 height f32), 4 gravity S `Vector3` |
| `GameStateChange` | 0 state, 1 is_overtime bool |
| `GameTimeChange` | 0 display_seconds i16 (the scoreboard clock) |
| `GoalSpawn` | 0 id S, 1 team, 2 center S, 3 direction S, 4 width f32, 5 height f32 |
| `PickupSpawn` | 0 id S, 1 location S `Vector3`, 2 type |
| `PickupStateChange` | 0 pickup_id S, 1 state |
| `PlayerSpawn` | 0 player_id S, 1 command_id S, 2 name string, 3 team, 4 car (T `CarBody`: 0 collision T `BoxShape` {length, width, height f32}, 1 offset S `Vector3`) |
| `PlayerPhysicsChange` | 0 player_id S, 1 physics S `PhysicsState`, 2 state `PlayerPhysicsState` |
| `PlayerBoostChange` | 0 player_id S, 1 is_active bool, 2 boost_amount u8 (0 to 100) |
| `BallSpawn` | 0 ball_id S, 1 command_id S, 2 collision_type tag `Shape`, 3 collision U (sphere: 0 diameter; cylinder: 0 diameter, 1 height; box: 0 length, 1 width, 2 height) |
| `BallPhysicsChange` | 0 ball_id S, 1 physics S |
| `BallHit` | 0 player_id S, 1 ball_id S, 2 impact_location S, 3 impact_normal S |
| `TeamScoreChange` | 0 team, 1 score u8 |
| `ActorDestroyed` | 0 id S |

Not decoded by `rb_rl_game_bridge` (named so a later reader knows they exist): the stat and
accolade messages, spectate, input echo, tile (Dropshot) and Rumble item messages, extra ball
data.

## RLBot to game commands

| Command | Slots |
|---|---|
| `PlayerInput` | 0 player_id S, 1 input (T `VehicleInput`) |
| `VehicleInput` | 0 throttle, 1 steer, 2 pitch, 3 yaw, 4 roll, 5 dodge_forward, 6 dodge_strafe f32; 7 jump, 8 boost, 9 handbrake, 10 use_item bool. All default 0 / false. (open) RLBot sets `dodge_forward = -pitch`, `dodge_strafe = yaw` (`FlatToModel.cs:19-22`). |
| `Exec` | 0 command string (a console command) |
| `SetPaused` | 0 paused bool |
| `EndMatch` | empty table |
| `DestroyActor` | 0 id S |
| `SetPhysics` | 0 id S, 1 physics S `PhysicsState` |
| `SetBoost` | 0 player_id S, 1 amount u8 |
| `SpawnBot` | 0 name string, 1 skill (`BotSkill`: Intro, Easy, Medium, Hard, Custom), 2 team, 3 loadout T (products, colours, paints) |
| render commands | line, line series, text 2D/3D, remove item |

Not encoded by `rb_rl_game_bridge`: `SpawnBot` (needs the loadout model) and the render
commands.

## Behaviour that is not in the bytes

- The game sends up to `RLBot_PacketSendRate` (240) messages per second; `delta_physics_frames`
  says how many 120 Hz physics frames each covers. About half of them leave game time unchanged
  (see `rb_rl_game_bridge::tick`).
- A player or ball is known by the `ActorID` of its spawn message. A car RLBot asked to spawn
  announces itself with the `command_id` of the `SpawnBot` that made it; RLBot uses that to
  tie its player index to the actor.
- A message may name an actor that is not spawned yet; the link keeps what it can and drops the
  rest rather than failing.

## Verification

Layouts were read from the decompiled accessors and builders, not guessed, then checked in
both directions against the DLL's own generated FlatBuffers code (a scratch project outside this
repo built `tests/fixtures/*.bin` with it):

- Rust decodes the DLL-built `GamePacket` (13 events) and `ControllerPacket` (7 commands) field
  for field (`rb_rl_game_bridge/tests/golden.rs`).
- The DLL's generated reader, pointed at what the Rust encoder writes, prints the same values
  (run once by hand, 2026-10-11; `RB_WRITE_ENCODED=<dir> cargo test -p rb_rl_game_bridge --test
  golden` writes the files).

**Not checked:** a capture from a running game. Whether the game really sends only these
events in this order, how it numbers actors, and whether the sizes fit its limits is unknown
until `rb_rl_game_probe` is run against it and the capture is kept as a fixture.
