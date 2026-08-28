# RB-VERIFY-001 — Replay Ingestion

- Version: 0.1.0
- Status: Draft
- Owners: baileyrd
- Depends on: none
- Supersedes: none

## Purpose and scope

Parse Rocket League `.replay` files into the domain's `PhysicsFrame`
sequence (`rb_domain::state`), so recorded real matches (online or offline)
can serve as one of the two ground-truth sources for divergence scoring
(`RB-VERIFY-003`).

In scope: parsing ball and car position/rotation/velocity/angular
velocity/boost state per frame, and whatever input signal (throttle,
steer, jump, boost, air-roll) the replay format actually exposes reliably.

## Non-goals

- Not a general-purpose replay analytics tool (stats, highlights, etc.) —
  only what the verification pipeline needs.
- Not expected to recover fully reliable raw controller inputs. Prior
  first-hand experience building a replay viewer/analyzer over ~1,000
  replay files confirms replay files give high-fidelity physics state but
  lossy/inferred-at-best input data. This adapter should surface whatever
  input signal exists and flag its own confidence, not pretend to a
  precision the source data doesn't have. See
  [docs/research/RESEARCH-BACKLOG.md](../../research/RESEARCH-BACKLOG.md).
- Not responsible for divergence scoring itself (`RB-VERIFY-003`) or for
  BakkesMod capture parsing (`RB-VERIFY-002`).

## Context and terminology

- **Replay file**: Rocket League's recorded-match format, parseable via the
  Rust crate `boxcars`.
- **PhysicsFrame**: this project's normalized per-tick state
  (`rb_domain::state::PhysicsFrame`) — the common representation both
  ingestion adapters produce.

## Requirements

- `RB-VERIFY-001-FR-001`: Given a valid `.replay` file, produce a
  chronologically ordered `Vec<PhysicsFrame>` covering the full match
  duration.
- `RB-VERIFY-001-FR-002`: Ball state (position, rotation, velocity, angular
  velocity) is extracted for every frame the replay records it.
- `RB-VERIFY-001-FR-003`: Car state (position, rotation, velocity, angular
  velocity, boost amount) is extracted per player, per frame.
- `RB-VERIFY-001-FR-004`: When raw controller input is recoverable from the
  replay, it is attached to the frame; when it isn't (the common case per
  the non-goals above), the frame's input field is `None`/absent rather
  than a fabricated value.
- `RB-VERIFY-001-NFR-001`: A malformed or truncated replay file produces
  `Err(IngestError::Malformed(_))`, never a panic.
- `RB-VERIFY-001-NFR-002`: Parsing a typical (~5-10 minute) replay completes
  in well under a second — this feeds an interactive verification loop, not
  a batch job.

## Architecture and interfaces

Implements `rb_domain::port::PhysicsStateSource` in the `rb_replay_ingest`
crate. `boxcars` is the parsing backend (not yet added as a dependency —
see Open Questions).

## Data/state and invariants

Output frames are timestamp-ordered and timestamps are relative to replay
start (seconds), matching `PhysicsFrame::timestamp_secs`'s contract.

## Errors, failure, recovery, and observability

`IngestError::Malformed` for parse failures, `IngestError::Io` for file
read failures, `IngestError::NotImplemented` until this spec's requirements
are actually implemented (current bootstrap state).

## Security, privacy, and compatibility

Replay files are the owner's own match recordings; no third-party data
handling concerns. No compatibility promise yet on `boxcars` version or
replay format version — track as it becomes relevant.

## Acceptance criteria

- Parses at least one real replay file end-to-end into `PhysicsFrame`s with
  correct ball position at a manually-verified timestamp.
- Malformed-input test produces `IngestError::Malformed`, not a panic.

## Verification plan

Unit tests against fixture replay files (small, checked-in or
generated) plus at least one real replay from the owner's own match
history, run manually and compared against BakkesMod/replay-viewer ground
truth before being trusted as a pipeline input.

## Traceability

See [docs/traceability/TRACEABILITY.md](../../traceability/TRACEABILITY.md).

## Open questions

- Exact `boxcars` API surface for input recovery — needs a spike before
  FR-004 can be implemented with confidence.
- Whether to vendor small fixture replay files in the repo or fetch them
  from a private location (real match replays may be large and are the
  owner's personal data).

## Change history

- 0.1.0 (2026-08-28): Initial draft, bootstrap phase.
