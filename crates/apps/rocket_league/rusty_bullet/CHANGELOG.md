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
### Changed
### Fixed
### Security

<!-- ## [0.1.0] - YYYY-MM-DD
### Added
- Initial release -->
