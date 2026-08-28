# Project Status

- Last verified main commit: *(pending first push to `main` — see note below)*
- Verified at: 2026-08-28
- Current milestone: `PHASE-1-PHYSICS-CORE-V0` (Bullet3 port, sphere-vs-plane)
- Health: green — workspace builds, `fmt`/`clippy`/`test` all pass

## Completed

- `PHASE-0-BOOTSTRAP` — charter, system architecture, spec tree (6 specs),
  spec registry, ADRs, research backlog, roadmap, traceability, AGENTS.md/
  WORKFLOW.md, governance file set (README/CONTRIBUTING/CODE_OF_CONDUCT/
  SECURITY/CHANGELOG/RELEASE_NOTES/PR & issue templates), CI workflow, and a
  minimal buildable Cargo workspace with the divergence-scoring algorithm
  implemented and unit-tested.
- `PHASE-1-PHYSICS-CORE-V0` — `rb_physics_bullet`, a from-scratch Rust port
  of Bullet3's rigid-body integration and sequential-impulse contact solver
  (zlib-licensed, see `THIRD_PARTY_NOTICES.md`), scoped to a dynamic sphere
  (ball) vs. static plane (ground) — per ADR-0004. Gravity, damping,
  semi-implicit Euler + exponential-map integration, restitution, and
  Coulomb friction all implemented and unit-tested (26 tests: free-fall
  kinematics, resting contact, bounce proportional to restitution, friction
  deceleration + spin coupling). `RB-RESEARCH-O001` (build vs. integrate)
  resolved by this decision.

Evidence for both: commits on `claude/rocket-league-server-clone-u74q45`
(not yet merged to `main`).

## In progress

- None — both units above are complete pending review/merge of this branch.

## Blocked

- `RB-RESEARCH-O002` (binary reverse engineering of the shipped Rocket
  League client) — blocked on two things: (1) explicit owner sign-off after
  a legal/practical review, and (2) practically, this sandboxed environment
  has no access to the Rocket League client binary at all, so any actual RE
  work would have to happen on the owner's own machine. See
  `docs/research/RESEARCH-BACKLOG.md`.

## Next

1. `PHASE-0-REPLAY-INGEST` — add `boxcars` as a dependency of
   `rb_replay_ingest` and implement `RB-VERIFY-001` against a real replay
   file from the owner's own match history. This (plus
   `PHASE-0-CAPTURE-INGEST`) is what will let `RB-PHYSICS-001-FR-005`
   (constant calibration) and real divergence scoring of the v0 physics
   core actually happen, rather than relying on unit tests alone.
2. `RB-PHYSICS-001-FR-004` — extend `rb_physics_bullet` to box-shaped car
   bodies (general 3x3 inertia, box collision, multi-contact resolution) —
   the next real increment of Phase 1, once ball physics has real data to
   validate against.

## Validation

- `cargo fmt --all -- --check`: pass
- `cargo clippy --workspace --all-targets -- -D warnings`: pass
- `cargo test --workspace`: pass (41 tests: 13 in `rb_domain`, 26 in
  `rb_physics_bullet`, 1 each in `rb_replay_ingest`/`rb_capture_ingest`, 0
  in `rb_verify_cli` (binary-only, no unit tests yet), plus doc-tests)

## Risks and decisions needed

- `RB-RESEARCH-O001` (build vs. integrate physics) — **resolved**, see
  ADR-0004. Revisit only if real divergence data later shows the approach
  underperforming (see ADR-0004's revisit trigger).
- `RB-RESEARCH-O002` (binary reverse engineering) — needs explicit owner
  sign-off after legal/practical review before any work starts, and needs
  the owner's own machine/game install since this sandbox has neither.
  Owner: baileyrd.
- `RB-RESEARCH-O003` (capture tooling scope) — decide at
  `PHASE-0-CAPTURE-INGEST` start. Owner: baileyrd.
- This is a fresh repo with no `main` history yet; the note above about
  "last verified main commit" should be filled in once this branch is
  merged.
