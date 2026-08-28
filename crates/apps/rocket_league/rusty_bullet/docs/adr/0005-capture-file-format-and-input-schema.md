# ADR-0005: JSON-Lines capture file format and a shared `ControllerInput` schema

- Status: Accepted
- Date: 2026-08-28
- Deciders: baileyrd
- Related: RB-VERIFY-002, RB-VERIFY-001-FR-004, RB-RESEARCH-O003
- Supersedes/Superseded by: none (resolves RB-VERIFY-001-FR-004's deferred
  domain-schema decision and RB-VERIFY-002's undesigned capture format;
  resolves RB-RESEARCH-O003)

## Context

`RB-VERIFY-002` (BakkesMod offline-capture ingestion) needs two things this
repo didn't have yet: a file format the (not-yet-written) BakkesMod-side
plugin writes and `rb_capture_ingest` reads, and a place on
`rb_domain::CarState` to attach the controller input that format carries.
`RB-VERIFY-001-FR-004` deferred exactly this schema question, on the
grounds it should be decided once, considering both adapters' real shapes,
not bolted onto `rb_replay_ingest` alone. Implementing `rb_capture_ingest`
for real is the point at which both shapes are actually known:
`subtr_actor::PlayerInputFrame` (raw replicated throttle/steer bytes, no
analog pitch/yaw/roll — see `rb_replay_ingest`'s non-goals) for replays, and
BakkesMod's own `ControllerInput` struct (full analog throttle/steer/pitch/
yaw/roll plus boolean flags) for captures.

`RB-RESEARCH-O003` (does the BakkesMod-side capture tool need to be a
reusable, versioned harness, or is a one-off script enough for Phase 0) is
also live here: the file format decision below is exactly the "committing
to a shape" moment that question was waiting on.

## Decision drivers

- `rb_domain` should stay dependency-free as long as possible (`AGENTS.md`)
  — a serialization format belongs in the adapter that needs it, not on the
  shared domain types.
- No existing binary-serialization dependency in the workspace to justify
  reusing; JSON via `serde`/`serde_json` is the ubiquitous default and
  ubiquitously debuggable (a capture file is something the owner will want
  to eyeball while iterating on the BakkesMod plugin).
- `RB-RESEARCH-O003`'s own stated default, absent evidence a harness is
  needed: the smaller option. A single, unversioned line format needs no
  format-negotiation machinery.
- Recorder crash-safety: a capture is written incrementally, tick by tick,
  during live play. A format where each tick is independently parseable
  (one JSON object per line) means a crash mid-write loses at most the
  final partial line, not the whole file.
- The two adapters recover genuinely different subsets of controller input
  (see Context) — the shared type has to represent "not recovered" honestly
  rather than default missing analog axes to `0.0`, which would silently
  claim "held neutral" for data that was actually just never captured.

## Considered options

1. **JSON Lines (JSONL), one `PhysicsFrame`-shaped object per tick, decoded
   via `serde_json`.** Human-readable, trivially appendable, no versioning
   machinery, malformed-line detection is just "this line didn't parse."
2. **A custom length-prefixed binary format.** More compact and faster to
   parse, but hand-rolled binary framing is exactly the kind of "substantial,
   error-prone parsing layer" `rb_replay_ingest`'s own dependency comment
   warns against reinventing — and there's no performance evidence yet
   (`RB-VERIFY-002-NFR-002` isn't benchmarked) that JSON parsing is
   actually too slow for a capture tool that only has to keep up with local
   offline play, not a live broadcast.
3. **Reuse `bincode`/`postcard` with `serde`'s `Serialize`/`Deserialize`
   derived directly on `rb_domain`'s types.** Faster and more compact than
   JSON, but couples `rb_domain` to a specific wire encoding and pulls a
   serialization dependency into the one crate this project has kept
   dependency-free the longest, for a format nothing has shown JSON too
   slow for.

## Decision

Adopt option 1. The capture file format is JSON Lines: each non-empty line
is one JSON object, `{"timestamp_secs", "ball", "cars"}`, matching
`rb_domain::PhysicsFrame`'s shape, with each entry in `cars` carrying an
`"input"` object. `rb_capture_ingest` owns `serde`/`serde_json` and a small
wire-format module with plain `struct`s mirroring the JSON shape (mirroring
`rb_replay_ingest::convert`'s own pattern of pure, independently-testable
conversion functions between a source-specific wire shape and
`rb_domain::state` types) — `rb_domain` itself gains no serialization
dependency. No format-version field yet: Phase 0 has exactly one writer
(the BakkesMod plugin, still to be built) and one reader
(`rb_capture_ingest`), evolved together; a version field is worth adding
the day a second format revision needs to be told apart from the first, not
before (`RB-RESEARCH-O003`'s "default to the smaller option" applies
directly). This resolves `RB-RESEARCH-O003` as "script, not harness" for
Phase 0.

`rb_domain::state::ControllerInput` (new type) is added with `throttle`/
`steer` as plain `f32` (both adapters can always produce a number, even if
only as a replicated byte for replay) and `pitch`/`yaw`/`roll` as
`Option<f32>` (only BakkesMod captures ever populate these — a replay's
dodge impulse/torque vectors are a different kind of quantity, not an
analog stick angle, and must not be reinterpreted as one). `jump`/`boost`/
`handbrake` are plain `bool`. `CarState` gains `input: Option<ControllerInput>`
— `None` when a source has no input data for that frame at all.

## Consequences

### Positive

- `RB-VERIFY-001-FR-004` and `RB-VERIFY-002-FR-001`/`FR-002`'s schema
  question are resolved by one decision instead of two adapters converging
  on incompatible shapes independently.
- `rb_replay_ingest` can now attach the input it already recovers (raw
  throttle/steer bytes, boost/jump/handbrake booleans) to `CarState`
  instead of discarding it after parsing — see the corresponding
  `convert.rs` change.
- The capture format is inspectable with any text editor/`jq`, which
  matters while the BakkesMod-side plugin (not yet written) is still being
  iterated on by the owner, off this sandbox.
- `rb_domain` gains no new dependency.

### Negative / tradeoffs

- JSON Lines is not the most compact or fastest option — acceptable per
  `RB-VERIFY-002-NFR-002`'s own unmeasured status; revisit if a real
  capture session shows parse time or file size actually mattering.
- `Option<f32>` on three of `ControllerInput`'s eight fields is more
  ceremony at every call site than a flat `f32` struct would be — accepted
  because defaulting an unrecoverable analog axis to `0.0` is a correctness
  problem (a fabricated "held neutral" reading), not just an ergonomics
  one.
- No format version field means a future incompatible format change has no
  built-in way to tell old and new capture files apart — accepted per
  `RB-RESEARCH-O003`'s explicit "revisit once a second format revision is
  needed" trigger, not overlooked.

## Validation and revisit triggers

Revisit the format choice if a real BakkesMod capture session (once the
plugin exists — see `RB-VERIFY-002`'s open questions) shows JSON parsing or
file size actually causing problems, or if a second capture format
revision needs versioning to coexist with the first. Revisit
`ControllerInput`'s shape if a third ingestion source needs to represent
input in a way these fields can't express.
