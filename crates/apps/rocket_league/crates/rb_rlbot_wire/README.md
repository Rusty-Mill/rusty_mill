# rb_rlbot_wire

The RLBot v5 socket protocol, hand-written on [`rusty_flatbuffers`](../../../../libs/protocol/rusty_flatbuffers).
Stage 2 of the native RLBot port (`../../rlbot/PLAN.md`).

- **Frames:** `frame::frame`, `frame::read_frame` (blocking) and `frame::FrameDecoder` (byte
  stream in arbitrary pieces, for non-blocking sockets): a `u16` big-endian length, then the payload.
- **Messages:** `InterfaceMessage` (client to core: connection settings, init complete, stop, player
  input, state setting, match configuration, ping response) and `CoreMessage` (core to client: game
  packet, field info, controllable team info, match configuration, disconnect signal, ping request;
  anything else is `Other(tag)`). Both encode **and** decode, so a stand-in core can be built from the same types.
- **Scope:** the fields `rb_tape_bot` and a match runner use. Each type documents what it leaves
  out (loadouts, 37 of 38 mutators, scripts, hitboxes, scores, ...). Fields the schema marks
  required are always written (valid placeholders for what is not modelled) and **required on
  read**: a payload missing one is `Error::Missing`, optional ones may be absent. A generator for the full schema is planned for when this
  outgrows hand-writing.
- **Verified against the reference:** `tests/golden.rs` decodes payloads made by `rlbot_flat` 0.6.0
  (schema rev `c38374e`, hex in `tests/fixtures/`), round-trips every message, and checks that
  damaged payloads return errors. `planus` (`rlbot_flat`) also accepts what this crate encodes for
  every client and core message; `tests/verified.rs` pins those exact bytes, and `tests/required.rs`
  checks required/optional slots on the wire bytes with its own small vtable reader.
