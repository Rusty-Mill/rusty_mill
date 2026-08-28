# Changelog

All notable changes to this repo are documented here.
Format: Added / Changed / Deprecated / Removed / Fixed / Security, newest first.

## [Unreleased]
### Added
- Repo lifecycle bootstrap: charter, system architecture, spec tree
  (`RB-VERIFY-001/002/003`, `RB-PHYSICS-001`, `RB-SIM-001`, `RB-NET-001`),
  ADR-0001..0003, research backlog, roadmap, traceability, AGENTS.md,
  WORKFLOW.md, and the standard governance file set.
- Cargo workspace (`rb_domain`, `rb_replay_ingest`, `rb_capture_ingest`,
  `rb_verify_cli`) with a working, unit-tested divergence-scoring algorithm
  and stubbed ingestion adapters.
- `rb_physics_bullet`: a Rust port of Bullet3's rigid-body integration and
  sequential-impulse contact solver, scoped to a dynamic sphere vs. a
  static plane (ADR-0004). Vector/quaternion algebra added to
  `rb_domain::state`.
- `rb_replay_ingest`: real `.replay` parsing via `boxcars` + `subtr-actor`
  (`RB-VERIFY-001-FR-001/002/003`), verified against a vendored real
  replay fixture.
- `rb_replay_ingest`: `corpus_check` bin, a local/gitignored-corpus health
  check (`RB-VERIFY-001-NFR-003`) — validated against 40 of the owner's
  real match replays (40/40 clean), closing the "runs correctly on real
  owner data at scale" half of `RB-VERIFY-001`'s owner-data acceptance
  criterion.
- `rb_domain::ControllerInput` and `CarState.input` (ADR-0005), a shared
  controller-input schema; `rb_replay_ingest` now attaches recovered input
  to every car (`RB-VERIFY-001-FR-004`).
- `rb_capture_ingest`: real capture-file parsing via a new JSON-Lines
  format (`RB-VERIFY-002-FR-002`/`NFR-001`, ADR-0005), verified against a
  synthetic fixture. The BakkesMod-side plugin that would write a real
  capture (`RB-VERIFY-002-FR-001`) is not yet built.
- `rb_verify_cli`: `score_replay_against_capture`, wiring ingestion to
  `rb_domain::divergence::score`. Manually run end-to-end against a real
  replay fixture and a capture file; not yet a fidelity measurement (see
  `RB-VERIFY-003`'s open FR-002/FR-003).
### Changed
- `rb_verify_cli`'s `main.rs` is now a thin CLI wrapper over the new
  `lib.rs`; `rb-verify`'s output is a human-readable summary instead of a
  raw `Debug` dump.
### Fixed
### Security

<!-- ## [0.1.0] - YYYY-MM-DD
### Added
- Initial release -->
