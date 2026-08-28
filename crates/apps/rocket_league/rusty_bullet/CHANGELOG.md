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
### Changed
### Fixed
### Security

<!-- ## [0.1.0] - YYYY-MM-DD
### Added
- Initial release -->
