# Release Notes

Tracks notable changes to this repo, one entry per merged change against
`main`, reverse chronological. Pre-1.0, no version tags yet — entries are
keyed by the commit/PR that shipped them.

---

## Physics core v0 — Bullet3 port (sphere vs. ground)
**2026-08-28** · commits on `claude/rocket-league-server-clone-u74q45` (not yet merged; links added once merged)

- **Added:** `rb_physics_bullet`, a from-scratch Rust port of specific
  Bullet3 (zlib-licensed) algorithms — rigid-body integration
  (`btRigidBody`) and the sequential-impulse contact solver
  (`btSequentialImpulseConstraintSolver`) — scoped to a dynamic sphere (the
  ball) against a static plane (the ground). Resolves the build-vs-integrate
  physics question via ADR-0004, ahead of `PHASE-0-EXIT` divergence data
  existing, on the strength of Bullet3's direct relevance and permissive
  license.
- **Added:** vector/quaternion algebra (dot, cross, normalize, quaternion
  product/rotation) on `rb_domain`'s `Vec3`/`Quat`, justified by the
  physics crate as a second real consumer.
- Known, deliberate scope cuts stated plainly: no car (box) rigid bodies or
  general 3x3 inertia tensor yet, no split impulse, no warm-starting or
  sleeping — a bouncy (restitution > 0) resting contact does not settle
  under this solver, by design of what v0 covers, not by accident. See
  `RB-PHYSICS-001` and `rb_physics_bullet::solver`'s module doc.
- Also completed the legal/practical review `RB-RESEARCH-O002` (binary
  reverse engineering of the shipped client) needed: Epic/Psyonix's EULA
  and Rocket League's Code of Conduct both contractually prohibit reverse
  engineering, and this sandbox has no access to the game binary regardless
  — still open pending the owner's own legal counsel and sign-off.
- 26 new unit tests (41 total in the workspace); `cargo fmt --check`,
  `clippy -D warnings`, and `cargo test --workspace` all pass.

## Repo bootstrap — full lifecycle baseline
**2026-08-28** · commit on `claude/rocket-league-server-clone-u74q45` (not yet merged; link added once merged)

- **Added:** Full `rust-repo-lifecycle` + `repo-config` bootstrap: charter,
  system architecture, a 6-spec tree (`RB-VERIFY-001/002/003` fully
  specified for Phase 0; `RB-PHYSICS-001`/`RB-SIM-001`/`RB-NET-001` as
  forward-looking placeholders), 3 ADRs (server-authoritative netcode,
  verification-first ordering, Bullet-fidelity target), a research backlog
  (6 settled findings + 3 tracked open questions), a Phase 0-4 roadmap with
  exit criteria tied to the divergence metric, requirement-level
  traceability, AGENTS.md/WORKFLOW.md, and the standard governance file set
  (README, CONTRIBUTING, CODE_OF_CONDUCT, SECURITY, CHANGELOG, PR/issue
  templates).
- **Added:** Minimal buildable Cargo workspace — `rb_domain` (physics frame
  types, `PhysicsStateSource` port, divergence-scoring algorithm),
  `rb_replay_ingest`/`rb_capture_ingest` (adapter stubs implementing the
  port), `rb_verify_cli` (composition-root binary). Divergence scoring
  (`RB-VERIFY-003-FR-001`) is real and unit-tested; both ingestion adapters
  are intentionally stubbed (`IngestError::NotImplemented`) — `boxcars`
  parsing and the BakkesMod capture format are Phase 0 delivery work, not
  bootstrap scaffolding.
- Known scope cut, stated plainly: no physics/simulation/netcode code
  exists yet — this PR is the governed baseline the rest of the project
  builds against, per ADR-0002's verification-first ordering.
- 11 unit tests added (6 in `rb_domain`, 1 each in the two adapter stubs,
  plus workspace doc-tests); `cargo fmt --check`, `clippy -D warnings`, and
  `cargo test --workspace` all pass.
