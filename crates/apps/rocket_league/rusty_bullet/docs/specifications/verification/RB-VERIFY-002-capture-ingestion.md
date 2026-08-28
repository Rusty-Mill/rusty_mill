# RB-VERIFY-002 — BakkesMod Offline Capture Ingestion

- Version: 0.1.0
- Status: Draft
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
  abstraction" convention. Whether it grows into a reusable capture harness
  or stays a one-off script is an explicit open question (see Open
  Questions / research backlog), not decided here.
- Not responsible for divergence scoring (`RB-VERIFY-003`) or replay
  parsing (`RB-VERIFY-001`).

## Context and terminology

- **BakkesMod**: third-party Rocket League modding platform; exposes a
  `ControllerInput` struct at high frequency in local/offline play.
- **Capture file**: this project's own recorded-input+state format, written
  by the BakkesMod-side plugin and read by `rb_capture_ingest`. Format is
  not yet designed (see Open Questions).

## Requirements

- `RB-VERIFY-002-FR-001`: The BakkesMod-side capture tool records, per
  physics tick during a local/offline match: raw controller input
  (throttle, steer, pitch, yaw, roll, jump, boost, handbrake, air-roll) and
  ball/car physics state, to a capture file.
- `RB-VERIFY-002-FR-002`: `rb_capture_ingest` parses a capture file into a
  chronologically ordered `Vec<PhysicsFrame>`, with input data attached
  (unlike `RB-VERIFY-001`'s frames, where input is usually absent).
- `RB-VERIFY-002-NFR-001`: A malformed/truncated capture file produces
  `Err(IngestError::Malformed(_))`, never a panic.
- `RB-VERIFY-002-NFR-002`: Capture recording must not perceptibly affect
  local match framerate/physics behavior — it is meant to produce clean
  ground truth, not itself perturb the thing being measured.

## Architecture and interfaces

Implements `rb_domain::port::PhysicsStateSource` in the `rb_capture_ingest`
crate. The BakkesMod-side recording tool is a separate deliverable (likely
outside the Rust workspace, since BakkesMod plugins are typically C++/its
own SDK) — tracked here but not part of the `rb_capture_ingest` crate
itself.

## Data/state and invariants

Same `PhysicsFrame` contract as `RB-VERIFY-001`, with the addition that
input fields are expected to be populated, not absent.

## Errors, failure, recovery, and observability

`IngestError::Malformed`/`IngestError::Io` as in `RB-VERIFY-001`;
`IngestError::NotImplemented` until this spec's requirements are
implemented (current bootstrap state — neither the capture-file format nor
the BakkesMod-side recorder exist yet).

## Security, privacy, and compatibility

Captures are the owner's own local play sessions; no third-party data
handling concerns. BakkesMod itself is third-party software the project
depends on but does not distribute or modify.

## Acceptance criteria

- A capture file recorded from a real local/offline match round-trips
  through `rb_capture_ingest` into `PhysicsFrame`s with input data present
  and physics state matching what BakkesMod's own overlay/logging reports
  at a manually-verified timestamp.
- Malformed-input test produces `IngestError::Malformed`, not a panic.

## Verification plan

Manual cross-check against BakkesMod's own debug overlay for a handful of
timestamps in an initial capture, before trusting the pipeline further.
Unit tests against fixture capture files once the format is designed.

## Traceability

See [docs/traceability/TRACEABILITY.md](../../traceability/TRACEABILITY.md).

## Open questions

- Capture file format is undesigned — likely a simple length-prefixed
  binary or newline-delimited JSON given the "one-off script vs. reusable
  harness" question is itself open (see research backlog).
- Whether the BakkesMod-side recorder lives in this repo, a companion repo,
  or as a gist/script — depends on the scope decision above.
- BakkesMod SDK/plugin API specifics needed to read `ControllerInput` at
  the required frequency without frame drops.

## Change history

- 0.1.0 (2026-08-28): Initial draft, bootstrap phase.
