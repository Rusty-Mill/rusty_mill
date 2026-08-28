# RB-VERIFY-001 — Replay Ingestion

- Version: 0.2.0
- Status: In Progress (FR-001/002/003 implemented and tested against a real
  replay; FR-004 — attaching recovered input — deferred, see Open Questions)
- Owners: baileyrd
- Depends on: none
- Supersedes: none

## Purpose and scope

Parse Rocket League `.replay` files into the domain's `PhysicsFrame`
sequence (`rb_domain::state`), so recorded real matches (online or offline)
can serve as one of the two ground-truth sources for divergence scoring
(`RB-VERIFY-003`).

In scope: parsing ball and car position/rotation/velocity/angular
velocity/boost state per frame. Attaching recovered controller input
(throttle, steer, jump, boost, air-roll) is in scope for this spec but
deferred to a follow-up increment — see Open Questions.

## Non-goals

- Not a general-purpose replay analytics tool (stats, highlights, etc.) —
  only what the verification pipeline needs.
- **Revises RB-RESEARCH-S004's original framing**: that finding said replay
  input data is "lossy/inferred at best," based on prior first-hand
  replay-viewer experience. Building this adapter surfaced a more precise
  picture: `subtr-actor` (the crate this adapter uses — see Architecture)
  recovers *some* real input state directly from the replay's replicated
  vehicle-input actor — raw throttle/steer bytes and boolean flags for
  boost/jump/double-jump/dodge/powerslide — not just physics-derived
  inference. That's more than S004 assumed, though still coarser than a
  live controller's full analog resolution (dodge direction is an impulse/
  torque vector, not raw stick position). Attaching it to `PhysicsFrame`
  is real work still to do (`RB-domain` has no input field yet) — see Open
  Questions — this non-goal is about accuracy expectations, not about
  whether it's technically recoverable.
- Not responsible for divergence scoring itself (`RB-VERIFY-003`) or for
  BakkesMod capture parsing (`RB-VERIFY-002`).

## Context and terminology

- **Replay file**: Rocket League's recorded-match format, parseable via the
  Rust crate `boxcars`.
- **`subtr-actor`**: a purpose-built crate (MIT, actively maintained) that
  resolves `boxcars`' raw per-actor network updates into higher-level
  frame-indexed ball/player state — see Architecture for why this adapter
  depends on it rather than reimplementing that resolution itself.
- **PhysicsFrame**: this project's normalized per-tick state
  (`rb_domain::state::PhysicsFrame`) — the common representation both
  ingestion adapters produce.

## Requirements

- `RB-VERIFY-001-FR-001` (implemented): Given a valid `.replay` file,
  produce a chronologically ordered `Vec<PhysicsFrame>` covering the full
  match duration. Verified against a real ~428-second, 12,029-frame replay
  (see Verification plan).
- `RB-VERIFY-001-FR-002` (implemented): Ball state (position, rotation,
  velocity, angular velocity) is extracted for every frame the replay
  records it; frames where the ball itself is unavailable are omitted
  (`rb_domain::PhysicsFrame` has no "no ball" representation — see Data/
  state and invariants).
- `RB-VERIFY-001-FR-003` (implemented): Car state (position, rotation,
  velocity, angular velocity, boost amount) is extracted per player, per
  frame; a car absent from a given frame is simply left out of that
  frame's `cars`, not an error.
- `RB-VERIFY-001-FR-004` (open): Attach recovered input (throttle/steer
  bytes, boost/jump/double-jump/dodge/powerslide booleans — all available
  from `subtr_actor::PlayerFrame::Data::input`/boolean fields already) to
  the output. Deferred because `rb_domain::CarState`/`PhysicsFrame` have no
  input field yet, and adding one is a domain-schema decision that should
  be made once (considering `RB-VERIFY-002`'s BakkesMod input too), not
  bolted on ad hoc here.
- `RB-VERIFY-001-NFR-001` (implemented): A malformed or truncated replay
  file produces `Err(IngestError::Malformed(_))`, never a panic. A missing
  file produces `Err(IngestError::Io(_))`.
- `RB-VERIFY-001-NFR-002` (not yet measured): Parsing a typical (~5-10
  minute) replay should complete in well under a second — this feeds an
  interactive verification loop, not a batch job. Not yet benchmarked;
  the real-fixture test (12,029 frames) currently runs in ~2.6s including
  test-harness/compile overhead, not isolated parse time.

## Architecture and interfaces

Implements `rb_domain::port::PhysicsStateSource` in the `rb_replay_ingest`
crate. `boxcars` parses the raw replay/network stream; `subtr_actor::ReplayDataCollector`
resolves that into frame-indexed ball/player `boxcars::RigidBody` state —
adopted rather than hand-rolling boxcars' actor-graph resolution (net-cache/
property-id resolution, quantized rotation decoding, replay-version
differences), which is a substantial, error-prone parsing layer with an
existing, permissively-licensed, purpose-built solution (see
`crates/rb_replay_ingest/Cargo.toml`'s dependency comment). `convert.rs`
maps `subtr_actor`'s types to `rb_domain::PhysicsFrame` — kept as pure,
independently-unit-tested functions since `subtr_actor`'s frame-container
types (`FrameData`/`BallData`/`PlayerData`) have no public constructors and
can only be exercised via a real parsed replay.

## Data/state and invariants

Output frames are timestamp-ordered and timestamps are relative to replay
start (seconds), matching `PhysicsFrame::timestamp_secs`'s contract.
`CarState.player_id` is a stable per-replay index (0, 1, 2, ... in
`subtr_actor`'s player order), **not** a platform account ID —
`boxcars::RemoteId` is a multi-platform enum (Steam/Epic/PlayStation/...)
with no single numeric form, and nothing downstream needs more than
per-replay car identity. Boost is normalized from the replay's raw 0-255
byte to a 0-100 percentage to match the in-game HUD convention.

## Errors, failure, recovery, and observability

`IngestError::Malformed` for parse failures (`boxcars::ParseError` or
`subtr_actor::SubtrActorError`, both converted via their `Display`/variant
message), `IngestError::Io` for file read failures.
`IngestError::NotImplemented` no longer applies to this adapter (removed
once FR-001/002/003 landed) — it still applies to `rb_capture_ingest`.

## Security, privacy, and compatibility

Real match replays are the owner's own data; no third-party data handling
concerns there. The vendored test fixture
(`crates/rb_replay_ingest/fixtures/subtr-actor-sample.replay`) is a
third-party replay used only to integration-test parsing — see
`fixtures/README.md` for provenance and why it does *not* satisfy this
spec's owner-data acceptance criterion. No compatibility promise yet on
`boxcars`/`subtr-actor` version or replay format version — track as it
becomes relevant.

## Acceptance criteria

- (Met, via the vendored fixture) Parses a real replay file end-to-end
  into `PhysicsFrame`s: 12,029 frames, ball position within plausible
  soccar field bounds on every frame, car data present on a subset of
  frames.
- (Still open) The same, against **the owner's own** replay file, with
  ball position cross-checked at a manually-verified timestamp — the
  fixture above proves the pipeline runs correctly on real replay bytes,
  not that a specific position is correct at a specific instant. This
  environment has no access to the owner's replay files; the owner would
  need to supply one (or run this locally) to close this criterion.
- (Met) Malformed-input test produces `IngestError::Malformed`, not a
  panic; missing-file test produces `IngestError::Io`.

## Verification plan

Unit tests (10, in `rb_replay_ingest`): pure conversion-function tests
(`convert.rs`, no file needed) plus file-level tests (missing file,
malformed file, and the real vendored fixture producing a non-empty,
bounds-sane frame sequence with car data present). The owner-data
cross-check described in Acceptance criteria is still needed before this
adapter's output should be trusted as a `RB-VERIFY-003` scoring input for
real accuracy conclusions — the current tests establish "runs correctly,"
not "matches a known-correct value."

## Traceability

See [docs/traceability/TRACEABILITY.md](../../traceability/TRACEABILITY.md).

## Open questions

- FR-004 (attaching recovered input) — needs an `rb_domain` schema
  decision (an input field on `CarState`, or a parallel structure) made
  jointly with `RB-VERIFY-002`, not decided here.
- NFR-002 (parse performance) — not yet benchmarked in isolation from test
  overhead.
- Whether the vendored third-party fixture is sufficient for ongoing CI, or
  whether additional/more diverse fixtures (different replay versions,
  game modes, player counts) should be added as they're found to matter.

## Change history

- 0.2.0 (2026-08-28): FR-001/002/003 implemented via `boxcars` +
  `subtr-actor`, tested against a real vendored replay fixture (12,029
  frames). Revises RB-RESEARCH-S004's input-recovery characterization.
  FR-004 deferred pending an `rb_domain` schema decision.
- 0.1.0 (2026-08-28): Initial draft, bootstrap phase.
