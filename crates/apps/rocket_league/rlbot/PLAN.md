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
| `rb_rlbot_wire` | `crates/apps/rocket_league/crates/` | The subset of RLBot messages as plain structs with `decode`/`encode`, hand-written on `rusty_flatbuffers`, plus the `u16` framing. Tier S. |
| `rb_rlbot_client` | same | Blocking TCP client (`std::net`, no `mio`/`kanal`), bot/script loop, state setting. Async only if a real need shows up. |
| `rb_rlbot_bridge` | same, later | Drives `rb_env` as a bot: observation from `GamePacket`, `ControllerInput` out. The coupling that justified one family. |

## Stages, each its own PR, each green on its own
1. **`rusty_flatbuffers` core + golden vectors.** Reader/verifier/builder; round-trip tests plus
   byte-for-byte fixtures captured once from `rlbot_flat` and committed (no runtime
   dependency on it).
2. **`rb_rlbot_wire` subset** covering what `rb_tape_bot` sends and receives; golden bytes per
   message; fuzz-style truncation tests (decode must return `Err`, never panic).
3. **`rb_rlbot_client`**: connect, handshake, packet loop, setting state, starting/stopping a
   match. Tested against an in-process fake core (a socket speaking the same frames).
4. **Port `rb_tape_bot` to it, drop `rlbot`, remove the workspace `exclude`**, retire the
   standalone `Cargo.lock`. Verified against the real game once (the same four log scenarios
   `rb_match_log` produces), since CI cannot run Rocket League.
5. **`rb_rlbot_bridge`**: `rb_env` as a bot.

## Risks
- **Schema drift.** RLBot's schema is versioned upstream (v5 here). Golden vectors pin 0.6.0;
  record the schema version in `rb_rlbot_wire` and add one new fixture per upgrade.
- **Hand-written subset vs codegen.** Hand-writing 35 types is cheap; 267 is not. If the
  subset grows past ~60 types, write a small `.fbs`-to-Rust generator instead (separate PR,
  ADR needed).
- **No CI for the game.** Stage 4's real-game check is manual and recorded in the PR.
- **Rollback.** Each stage only adds crates until stage 4; stage 4 is one revertable PR.

## Decisions (owner, 2026-10-09)
1. Scope: through stage 5, the `rb_env` bridge.
2. Codec: hand-written subset now; a `.fbs`-to-Rust generator later (its own PR and ADR, once the subset outgrows hand-writing).
3. Naming: `rb_rlbot_*` for the family crates; the reusable runtime stays `rusty_flatbuffers`.

## Progress
- Stage 1: `rusty_flatbuffers` (reader, builder, golden fixtures from `rlbot_flat` schema rev c38374e). Cross-checked both ways: its reader reads planus-built buffers (committed fixtures), and planus reads its builder's output (checked once in a scratch crate).
- Stage 2: `rb_rlbot_wire` (framing, 6 client messages, 4 core messages, both directions). Golden payloads from `rlbot_flat` decode to the expected values; `planus` accepts what this crate encodes for all six client messages. Not yet covered: `RenderGroup`, `StartCommand`, `SetLoadout`, comms, ball prediction, pings (core pings arrive as `CoreMessage::Other`).
- Stage 3: `rb_rlbot_client` (`Connection` with handshake and timeouts, `run_bots` / `run_hivemind`, `Environment`), tested against an in-process fake core. `rb_rlbot_wire` gained disconnect and ping messages for it. Blocking and single-threaded on purpose (see its README); not yet run against the real core, which is stage 4's check.
- Stage 4: `rb_tape_bot` (bot, hive, probe, match log, tape runner) ported to `rb_rlbot_client` / `rb_rlbot_wire`; `rlbot` and `rlbot_flat` are gone from the dependency graph, the package is a workspace member, and its standalone `Cargo.lock` is retired. A tool-local `.cargo/config.toml` keeps the build output in `tools/rb_tape_bot/target`, where the bot configs launch it. Verified here by running the real `rb_tape_bot` and `rb_tape_hive` processes against a stand-in core (`tests/processes.rs`); the one run against the real game is the checklist at the end of the tool's README, to be recorded in the PR.
