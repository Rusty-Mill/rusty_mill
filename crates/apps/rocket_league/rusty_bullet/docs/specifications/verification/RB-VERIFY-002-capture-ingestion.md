# RB-VERIFY-002 — BakkesMod Offline Capture Ingestion

- Version: 0.4.0
- Status: In Progress (FR-001 — the BakkesMod-side plugin — built, loaded,
  and run against a real Rocket League + BakkesMod install; FR-002/NFR-001
  implemented and now also verified against that real capture, not just the
  synthetic fixture; NFR-002 still unmeasured, see Open Questions)
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

- `RB-VERIFY-002-FR-001` (implemented, verified): The BakkesMod-side
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
  paraphrase. Built with MSVC (VS2022 Build Tools) + CMake against the
  owner's own installed SDK copy, loaded into a real Rocket League +
  BakkesMod session, and run in freeplay: the hook fires correctly and the
  ball's live physics state records correctly, but the first real capture
  surfaced a genuine bug — enumerating cars via `ServerWrapper::GetPRIs()` +
  `PriWrapper::GetCar()` returned a frozen, all-zero-input car on every
  line, because a PRI's `Car` back-reference never gets updated in freeplay
  (PRI exists for scoreboard/stat tracking, which freeplay has none of).
  Fixed by switching to `ServerWrapper::GetCars()` (inherited via
  `GameEventWrapper`), the game's own live spawned-car-actor list. A second
  real capture (2,818 lines, ~23.5s) confirmed both ball and car state
  update correctly with real, varied controller input. See Change history.
- `RB-VERIFY-002-FR-002` (implemented, verified): `rb_capture_ingest`
  parses a capture file into a chronologically ordered `Vec<PhysicsFrame>`,
  with input data attached via `rb_domain::CarState.input` (unlike
  `RB-VERIFY-001`'s frames, where replay-sourced `pitch`/`yaw`/`roll` are
  always `None`). Verified against both a synthetic, hand-authored fixture
  (see `crates/rb_capture_ingest/fixtures/README.md`) and, now that FR-001
  exists, a real BakkesMod capture (2,818 frames, every car entry carrying
  `Some` input, chronologically ordered, 1,612 ticks with non-zero
  throttle/steer confirming real recorded driving) via a scratch
  integration test, not kept in the repo since the underlying capture file
  is the owner's own personal play data.
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

- (Met, for the round-trip half; the overlay cross-check half is still
  open) A capture file recorded from a real local/offline match round-trips
  through `rb_capture_ingest` into `PhysicsFrame`s with input data present:
  confirmed against two real captures, the second (2,818 lines, ~23.5s of
  freeplay) schema-validated line-by-line against ADR-0005 and parsed
  end-to-end by `rb_capture_ingest` with every car entry carrying `Some`
  input and real, varied throttle/steer/jump values. Not yet done: pinning
  one specific timestamp's physics state against what BakkesMod's own
  overlay/logging reports for that same instant — a manual owner cross-check,
  same shape as `RB-VERIFY-001`'s equivalent still-open item.
- (Met, via the synthetic fixture, now also confirmed against a real
  capture) `rb_capture_ingest` parses a well-formed JSON-Lines capture file
  end-to-end into `PhysicsFrame`s with every car's `input` populated, in
  chronological order.
- (Met) Malformed-line test produces `IngestError::Malformed`, not a
  panic; missing-file test produces `IngestError::Io`.

## Verification plan

Unit tests (10, in `rb_capture_ingest`): pure wire-conversion tests
(`wire.rs`, no file needed) plus file-level tests (missing file, malformed
line, blank lines skipped, and the synthetic fixture producing a
chronologically ordered, input-attached frame sequence). Additionally, two
manual real-capture verification passes on the owner's own machine (see
Change history): a Python schema check against every line of a real
capture, and a scratch `rb_capture_ingest` integration test (not kept in
the repo, per Security/privacy) confirming the same file parses end-to-end.
No manual BakkesMod-overlay single-timestamp cross-check has happened yet
(see Acceptance criteria).

## Traceability

See [docs/traceability/TRACEABILITY.md](../../traceability/TRACEABILITY.md).

## Open questions

- Manual BakkesMod-overlay single-timestamp cross-check (see Acceptance
  criteria) — still open, needs the owner to pin one instant against
  BakkesMod's own overlay/logging.
- NFR-002 (recording overhead) — still unmeasured; the plugin has now run
  in real matches without any observed framerate/physics impact, but this
  hasn't been rigorously benchmarked.
- Resolved: the hookable event name
  (`Function TAGame.Car_TA.SetVehicleInput`) is confirmed correct — it
  fired reliably across two real captures (9,358 and 2,818 lines), with the
  ball's `GetPhysicsFrame()`-based per-tick dedup producing exactly one line
  per tick throughout.
- Resolved: the JSON-Lines format (ADR-0005) is confirmed ergonomic to emit
  from BakkesMod's C++ SDK — both real captures schema-validated exactly
  against ADR-0005's shape with zero errors across thousands of lines.

## Change history

- 0.4.0 (2026-09-02): FR-001's plugin built (MSVC/VS2022 Build Tools +
  CMake, against the owner's own installed `BakkesModSDK` copy), loaded
  into a real Rocket League + BakkesMod session, and run in freeplay — the
  one step this sandbox couldn't do. First real capture (9,358 lines)
  proved the hook fires correctly and the ball's live state records
  correctly, but surfaced a genuine bug: enumerating cars via
  `ServerWrapper::GetPRIs()` + `PriWrapper::GetCar()` recorded the same
  frozen spawn-point transform and all-zero input on every line, because a
  PRI's `Car` back-reference is never updated in freeplay (PRI exists for
  scoreboard/stat tracking, which freeplay has none of) — confirmed by the
  ball's own independently-recorded velocity spiking mid-session (something
  clearly hit it) while the "car" entry never moved. Fixed by switching to
  `ServerWrapper::GetCars()` (inherited via `GameEventWrapper`), the game's
  own live spawned-car-actor list; rebuilt, redeployed, and re-verified with
  a second real capture (2,818 lines, ~23.5s) showing both ball and car
  state updating correctly with real, varied controller input (1,612 of
  2,818 ticks with non-zero throttle/steer). Every line of the second
  capture schema-validated exactly against ADR-0005, and the whole file
  parsed end-to-end via `rb_capture_ingest` (via a scratch integration test,
  not kept in the repo — see Security/privacy) with every car entry
  carrying `Some` input in chronological order. Resolves both of `0.3.0`'s
  hook-name and format-ergonomics open questions. FR-001 and FR-002 are now
  both implemented and verified; NFR-002 remains unmeasured and the
  manual overlay single-timestamp cross-check remains open.
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
