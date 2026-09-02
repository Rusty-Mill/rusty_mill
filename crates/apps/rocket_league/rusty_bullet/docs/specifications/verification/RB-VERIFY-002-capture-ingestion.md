# RB-VERIFY-002 — BakkesMod Offline Capture Ingestion

- Version: 0.3.0
- Status: In Progress (FR-002/NFR-001 implemented and tested against a
  synthetic fixture; FR-001 — the BakkesMod-side plugin — source written in
  `bakkesmod-plugin/rusty_bullet_capture/`, not yet built or run against a
  real game, see Open Questions)
- Owners: baileyrd
- Depends on: none
- Supersedes: none

## Purpose and scope

Capture and parse true raw controller input paired with resulting physics
state from BakkesMod, in local/offline matches only, to produce a clean
input→physics ground-truth set for divergence scoring (`RB-VERIFY-003`).

In scope: (a) a BakkesMod-side plugin/script that records `ControllerInput`
at high frequency alongside ball/car physics state to a capture file, and
(b) an `rb_capture_ingest` parser that turns that file into
`rb_domain::state::PhysicsFrame`s (with input attached, unlike
`RB-VERIFY-001`).

## Non-goals

- Not usable for online-match ground truth. Rocket League's Easy
  Anti-Cheat now blocks BakkesMod during online matches — this source is
  local/offline play only, by construction, not by choice. See
  [docs/research/RESEARCH-BACKLOG.md](../../research/RESEARCH-BACKLOG.md).
- Not a general BakkesMod plugin platform — the capture tool does exactly
  what this pipeline needs and no more, per the "no speculative
  abstraction" convention. `RB-RESEARCH-O003` is now resolved: a one-off
  script, not a reusable harness (see ADR-0005).
- Not responsible for divergence scoring (`RB-VERIFY-003`) or replay
  parsing (`RB-VERIFY-001`).

## Context and terminology

- **BakkesMod**: third-party Rocket League modding platform; exposes a
  `ControllerInput` struct at high frequency in local/offline play.
- **Capture file**: this project's own recorded-input+state format, written
  by the BakkesMod-side plugin and read by `rb_capture_ingest`. JSON Lines,
  one JSON object per physics tick — see Architecture and
  [ADR-0005](../../adr/0005-capture-file-format-and-input-schema.md).

## Requirements

- `RB-VERIFY-002-FR-001` (implemented, unverified): The BakkesMod-side
  capture tool records, per physics tick during a local/offline match: raw
  controller input (throttle, steer, pitch, yaw, roll, jump, boost,
  handbrake) and ball/car physics state, to a capture file matching
  ADR-0005's JSON-Lines format. Written as
  `bakkesmod-plugin/rusty_bullet_capture/` (a `BakkesModPlugin` hooking
  `Function TAGame.Car_TA.SetVehicleInput` post, deduped per tick via the
  ball's own `GetPhysicsFrame()` counter, with `rb_capture_start`/
  `rb_capture_stop` console commands), grounded against a real clone of
  [BakkesModSDK](https://github.com/bakkesmodorg/BakkesModSDK) (exact struct
  fields, wrapper hierarchy, and hookable-caller types), not against
  paraphrase. Still blocked on the owner's own Windows/BakkesMod/game
  environment for the one thing that can't happen in this sandbox: actually
  compiling and running it — see Open Questions and the plugin's own
  README.md.
- `RB-VERIFY-002-FR-002` (implemented): `rb_capture_ingest` parses a
  capture file into a chronologically ordered `Vec<PhysicsFrame>`, with
  input data attached via `rb_domain::CarState.input` (unlike
  `RB-VERIFY-001`'s frames, where replay-sourced `pitch`/`yaw`/`roll` are
  always `None`). Verified against a synthetic, hand-authored fixture (see
  `crates/rb_capture_ingest/fixtures/README.md`) — not yet against a real
  capture, since none exists (FR-001 not started).
- `RB-VERIFY-002-NFR-001` (implemented): A malformed capture line produces
  `Err(IngestError::Malformed(_))`, never a panic. A missing file produces
  `Err(IngestError::Io(_))`.
- `RB-VERIFY-002-NFR-002` (not yet measurable): Capture recording must not
  perceptibly affect local match framerate/physics behavior — inherently
  untestable until FR-001's plugin exists and runs in a real match.

## Architecture and interfaces

Implements `rb_domain::port::PhysicsStateSource` in the `rb_capture_ingest`
crate. The capture file is JSON Lines (one `{"timestamp_secs", "ball",
"cars"}` object per tick, each car entry carrying an `"input"` object) —
see [ADR-0005](../../adr/0005-capture-file-format-and-input-schema.md) for
why JSON Lines was chosen over a custom binary format. `wire.rs` holds the
`serde`-derived wire types and pure conversion into `rb_domain::state`,
mirroring `rb_replay_ingest::convert`'s own split so these functions are
unit-testable without a file on disk. The BakkesMod-side recording tool
itself is a separate deliverable outside the Rust workspace and outside
Cargo's build graph entirely (C++ against BakkesMod's own SDK) — see
`bakkesmod-plugin/rusty_bullet_capture/` — tracked here but not part of the
`rb_capture_ingest` crate itself (see Open Questions for its remaining
build/run verification).

## Data/state and invariants

Same `PhysicsFrame` contract as `RB-VERIFY-001`, with the addition that
`CarState.input` is expected to always be `Some` with every field
populated (unlike replay-sourced input, where `pitch`/`yaw`/`roll` are
structurally `None` — see ADR-0005).

## Errors, failure, recovery, and observability

`IngestError::Malformed` for a line that fails JSON decoding or doesn't
match the expected shape (missing required field, wrong type), reported
with a 1-based line number. `IngestError::Io` for file-open/read failures.
`IngestError::NotImplemented` no longer applies to this adapter's Rust-side
parsing (FR-002 is implemented) — it would still describe the BakkesMod
plugin's absence if that were represented in this crate, but the plugin is
a separate deliverable outside it (see Architecture).

## Security, privacy, and compatibility

Captures are the owner's own local play sessions; no third-party data
handling concerns. BakkesMod itself is third-party software the project
depends on but does not distribute or modify. The synthetic fixture used
for testing (`crates/rb_capture_ingest/fixtures/example.capture.jsonl`) is
hand-authored, not real game data — see `fixtures/README.md`.

## Acceptance criteria

- (Not yet met — FR-001's plugin is written but unbuilt/unrun) A capture
  file recorded from a real local/offline match round-trips through
  `rb_capture_ingest` into `PhysicsFrame`s with input data present and
  physics state matching what BakkesMod's own overlay/logging reports at a
  manually-verified timestamp.
- (Met, via the synthetic fixture) `rb_capture_ingest` parses a
  well-formed JSON-Lines capture file end-to-end into `PhysicsFrame`s with
  every car's `input` populated, in chronological order. This proves the
  parser is correct against the format's own schema — it does not prove
  the schema matches what a real BakkesMod plugin would produce, since none
  exists yet.
- (Met) Malformed-line test produces `IngestError::Malformed`, not a
  panic; missing-file test produces `IngestError::Io`.

## Verification plan

Unit tests (10, in `rb_capture_ingest`): pure wire-conversion tests
(`wire.rs`, no file needed) plus file-level tests (missing file, malformed
line, blank lines skipped, and the synthetic fixture producing a
chronologically ordered, input-attached frame sequence). No manual
BakkesMod-overlay cross-check has happened yet — that requires actually
building and running FR-001's plugin in a real local match, which hasn't
happened yet (see Open Questions).

## Traceability

See [docs/traceability/TRACEABILITY.md](../../traceability/TRACEABILITY.md).

## Open questions

- FR-001's plugin source exists (`bakkesmod-plugin/rusty_bullet_capture/`)
  but still needs to be built, loaded into a real Rocket League + BakkesMod
  install, and run through at least one local/offline match — none of
  which this sandboxed environment can do (no Windows, no BakkesMod, no
  game). Building/running is the one genuinely remaining step; it has to
  happen on the owner's own machine (see the plugin's own README.md).
- The hookable event name used
  (`Function TAGame.Car_TA.SetVehicleInput`) is grounded in the
  well-established BakkesMod ecosystem convention and in
  `CarWrapper`/`VehicleWrapper`'s own `eventSetVehicleInput` member (both
  confirmed against a real `BakkesModSDK` clone), but — unlike every other
  fact this spec cites — Unreal reflection event *names* aren't enumerated
  anywhere in the C++ SDK headers themselves, so this one specific string
  is unverified until the plugin actually loads and the hook fires in a
  real match.
- Whether the JSON-Lines format (ADR-0005) actually matches what's
  ergonomic to emit from BakkesMod's C++ SDK — the plugin's `wire.rs`-
  mirroring JSON builder (see `carJson`/`rbActorJson` in
  `RustyBulletCapturePlugin.cpp`) suggests it's straightforward, but this is
  still unconfirmed until a real capture round-trips through
  `rb_capture_ingest`.
- NFR-002 (recording overhead) — can't be measured without the plugin
  running in a real match.

## Change history

- 0.3.0 (2026-09-02): FR-001's plugin source written —
  `bakkesmod-plugin/rusty_bullet_capture/` (`RustyBulletCapturePlugin.h/.cpp`,
  `CMakeLists.txt`, `README.md`), grounded against a real clone of
  BakkesModSDK (exact `ControllerInput`/`RBState`/`ArrayWrapper` fields, the
  `ServerWrapper`→`GameEventWrapper`→`ActorWrapper` hierarchy for `.IsNull()`,
  and the `HookEventWithCallerPost<CarWrapper>` explicit instantiation).
  Deduplicates per-tick via `BallWrapper::GetPhysicsFrame()` since the
  chosen hook fires once per car per tick. Not yet built, loaded, or run —
  that requires the owner's own Windows/BakkesMod/Rocket League environment,
  which this sandbox doesn't have. Status is "implemented, unverified", not
  "done".
- 0.2.0 (2026-08-28): FR-002/NFR-001 implemented — `rb_capture_ingest`
  parses the JSON-Lines capture format (ADR-0005) into `PhysicsFrame`s with
  `CarState.input` populated, tested against a synthetic fixture (10 unit
  tests). FR-001 (the BakkesMod-side plugin) remains not started, blocked
  on the owner's own Windows/BakkesMod/game environment. Resolves
  `RB-RESEARCH-O003` (script, not harness) via ADR-0005.
- 0.1.0 (2026-08-28): Initial draft, bootstrap phase.
