# Project Status

- Last verified main commit: `93ad0e9` (merge of [#3](https://github.com/baileyrd/rusty_bullet/pull/3))
- Verified at: 2026-08-28
- Current milestone: `PHASE-0-REPLAY-INGEST` (boxcars + subtr-actor replay parsing) — merged
- Health: green — workspace builds, `fmt`/`clippy`/`test` all pass on `main`

## Completed

- `PHASE-0-BOOTSTRAP` — charter, system architecture, spec tree, spec
  registry, ADRs, research backlog, roadmap, traceability, AGENTS.md/
  WORKFLOW.md, governance file set, CI workflow, and a minimal buildable
  Cargo workspace with the divergence-scoring algorithm implemented and
  unit-tested. Merged via [PR #1](https://github.com/baileyrd/rusty_bullet/pull/1).
- `PHASE-1-PHYSICS-CORE-V0` — `rb_physics_bullet`, a from-scratch Rust port
  of Bullet3's rigid-body integration and sequential-impulse contact solver
  (zlib-licensed, see `THIRD_PARTY_NOTICES.md`), scoped to a dynamic sphere
  (ball) vs. static plane (ground) — per ADR-0004. Merged via
  [PR #1](https://github.com/baileyrd/rusty_bullet/pull/1).
- Status/workflow sync — merged via [PR #2](https://github.com/baileyrd/rusty_bullet/pull/2).
- `RB-VERIFY-001-FR-001/002/003` — `rb_replay_ingest` now really parses
  `.replay` files: `boxcars` parses the replay/network stream,
  `subtr-actor` resolves it into frame-indexed ball/car `RigidBody` state
  (avoiding a hand-rolled actor-graph resolver — see the crate's
  `Cargo.toml` dependency comment), and `convert.rs` maps that to
  `rb_domain::PhysicsFrame`. Verified end-to-end against a real vendored
  replay fixture (12,029 frames, ~428s match, ball position sane on every
  frame). Merged via [PR #3](https://github.com/baileyrd/rusty_bullet/pull/3).

## In progress

- None.

## Blocked

- `RB-RESEARCH-O002` (binary reverse engineering of the shipped Rocket
  League client) — blocked on two things: (1) explicit owner sign-off after
  a legal/practical review, and (2) practically, this sandboxed environment
  has no access to the Rocket League client binary at all, so any actual RE
  work would have to happen on the owner's own machine. See
  `docs/research/RESEARCH-BACKLOG.md`.
- `RB-VERIFY-001`'s owner-data acceptance criterion (ball position
  cross-checked against a real match at a manually-verified timestamp) —
  this environment has no access to the owner's own replay files. The
  vendored third-party fixture proves the pipeline runs correctly on real
  replay bytes, not that a specific position is correct at a specific
  instant. Owner would need to supply a replay file (or run this locally)
  to close this out.

## Next

1. `PHASE-0-CAPTURE-INGEST` — BakkesMod offline capture ingestion
   (`RB-VERIFY-002`). Needs the owner's own local/offline capture, since
   this sandbox can't run Rocket League or BakkesMod.
2. `RB-VERIFY-001-FR-004` / `RB-VERIFY-002` input schema — decide how
   recovered input (replay-derived throttle/steer/booleans; BakkesMod
   `ControllerInput`) attaches to `rb_domain`'s types, once both adapters'
   real shapes are known.
3. `RB-PHYSICS-001-FR-004` — extend `rb_physics_bullet` to box-shaped car
   bodies (general 3x3 inertia, box collision, multi-contact resolution).

## Validation

- `cargo fmt --all -- --check`: pass
- `cargo clippy --workspace --all-targets -- -D warnings`: pass
- `cargo test --workspace`: pass (51 tests: 13 in `rb_domain`, 26 in
  `rb_physics_bullet`, 10 in `rb_replay_ingest` (incl. real-fixture
  integration test), 1 in `rb_capture_ingest`, 0 in `rb_verify_cli`
  (binary-only, no unit tests yet), plus doc-tests)

## Risks and decisions needed

- `RB-RESEARCH-O001` (build vs. integrate physics) — **resolved**, see
  ADR-0004.
- `RB-RESEARCH-O002` (binary reverse engineering) — needs explicit owner
  sign-off after legal/practical review before any work starts, and needs
  the owner's own machine/game install since this sandbox has neither.
  Owner: baileyrd.
- `RB-RESEARCH-O003` (capture tooling scope) — decide at
  `PHASE-0-CAPTURE-INGEST` start. Owner: baileyrd.
- `RB-VERIFY-001-FR-004` input schema — decide jointly with
  `RB-VERIFY-002`, not before both adapters' real shapes are known. Owner:
  baileyrd.
