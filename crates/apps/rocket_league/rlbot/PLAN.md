# Native RLBot port: plan

Status: **proposal, no code.** Replaces the external `rlbot` 0.6.0 client (and its generated
`rlbot_flat`) so `rb_tape_bot` can join the workspace (ADR-0008, ADR-0002).

## What exists today (measured, rlbot 0.6.0)
- Wire protocol is small: TCP to RLBot core (`127.0.0.1:23234`), each message is a `u16`
  big-endian length followed by a FlatBuffers payload. `InterfaceMessage` goes out,
  `CoreMessage` comes in. The client crate is ~940 lines (connection, agent loops, state
  builder, render helpers) on `mio` + `kanal`.
- The weight is `rlbot_flat`: 46k generated lines (planus, 267 types) from the RLBot schema.
- `rb_tape_bot` uses ~35 of them: `ConnectionSettings`, `InitComplete`, `CoreMessage`,
  `GamePacket`, `FieldInfo`, `PlayerInput`, `ControllerState`, `DesiredGameState` and its
  ball/car/physics parts, `MatchConfiguration` and its mutators, `StopCommand`, plus the
  agent traits (`BotAgent`, `HivemindAgent`, `PacketQueue`).

## Target layout (ADR-0008: reusable pieces are libs, the rest is family)
| Piece | Home | Notes |
|---|---|---|
| `rusty_flatbuffers` | `crates/libs/protocol/` | Zero-dep runtime: table/vtable reader, verifier, builder. No codegen at first. |
| `rlbot_wire` | `crates/apps/rocket_league/crates/` | The subset of RLBot messages as plain structs with `decode`/`encode`, hand-written on `rusty_flatbuffers`, plus the `u16` framing. Tier S. |
| `rlbot_client` | same | Blocking TCP client (`std::net`, no `mio`/`kanal`), bot/script loop, state setting. Async only if a real need shows up. |
| `rlbot_bridge` | same, later | Drives `rb_env` as a bot: observation from `GamePacket`, `ControllerInput` out. The coupling that justified one family. |

## Stages, each its own PR, each green on its own
1. **`rusty_flatbuffers` core + golden vectors.** Reader/verifier/builder; round-trip tests plus
   byte-for-byte fixtures captured once from `rlbot_flat` and committed (no runtime
   dependency on it).
2. **`rlbot_wire` subset** covering what `rb_tape_bot` sends and receives; golden bytes per
   message; fuzz-style truncation tests (decode must return `Err`, never panic).
3. **`rlbot_client`**: connect, handshake, packet loop, setting state, starting/stopping a
   match. Tested against an in-process fake core (a socket speaking the same frames).
4. **Port `rb_tape_bot` to it, drop `rlbot`, remove the workspace `exclude`**, retire the
   standalone `Cargo.lock`. Verified against the real game once (the same four log scenarios
   `rb_match_log` produces), since CI cannot run Rocket League.
5. **`rlbot_bridge`**: `rb_env` as a bot.

## Risks
- **Schema drift.** RLBot's schema is versioned upstream (v5 here). Golden vectors pin 0.6.0;
  record the schema version in `rlbot_wire` and add one new fixture per upgrade.
- **Hand-written subset vs codegen.** Hand-writing 35 types is cheap; 267 is not. If the
  subset grows past ~60 types, write a small `.fbs`-to-Rust generator instead (separate PR,
  ADR needed).
- **No CI for the game.** Stage 4's real-game check is manual and recorded in the PR.
- **Rollback.** Each stage only adds crates until stage 4; stage 4 is one revertable PR.

## Decisions needed before stage 1
1. Scope: client only (stages 1-4), or through `rlbot_bridge` (stage 5)?
2. Codec: hand-written subset (recommended) or a generator from the start?
3. Naming: `rlbot_*` as above (keeps the family's `rb_*` for physics), or `rb_rlbot*`?
