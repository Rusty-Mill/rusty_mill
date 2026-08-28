# Project Status

- Last verified main commit: `f10d017` (merge of [#9](https://github.com/baileyrd/rusty_bullet/pull/9))
- Verified at: 2026-08-28
- Current milestone: `PHASE-0-EXIT` (`rb_verify_cli` wires ingestion to divergence scoring; runs end-to-end mechanically, not yet a real fidelity comparison) — In Progress
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
- `RB-VERIFY-001-NFR-003` — a local, gitignored corpus health-check bin
  (`corpus_check`), run once against 40 of the owner's own real match
  replays (`baileyrd/replays`): 40/40 parsed cleanly, sane ball-position
  bounds on every file. Closes the "runs correctly on real owner data at
  scale" half of `RB-VERIFY-001`'s owner-data acceptance criterion; the
  manual single-timestamp cross-check remains open (see Blocked). Marks
  `PHASE-0-REPLAY-INGEST` Done.
- `ADR-0005` — decided the capture file format (JSON Lines) and a shared
  `rb_domain::ControllerInput` schema, resolving `RB-RESEARCH-O003` and the
  domain-schema question `RB-VERIFY-001-FR-004` had deferred.
  `RB-VERIFY-001-FR-004` — `rb_replay_ingest` now attaches recovered input
  (throttle/steer/jump/boost/handbrake) to `CarState.input`; `pitch`/`yaw`/
  `roll` stay `None` for replay-sourced input (never recoverable from a
  replay, see ADR-0005).
- `RB-VERIFY-002-FR-002`/`NFR-001` — `rb_capture_ingest` parses the
  JSON-Lines capture format into `PhysicsFrame`s with `CarState.input`
  always populated, tested against a synthetic hand-authored fixture (no
  real BakkesMod capture exists — see Blocked).
- `rb_verify_cli` divergence-scoring CLI wiring — `score_replay_against_capture`
  (new `lib.rs`, `main.rs` is now a thin argument/output wrapper over it)
  ingests a replay + a capture and runs `rb_domain::divergence::score`.
  Manually run against the vendored replay fixture + synthetic capture
  fixture: `frames compared: 5, mean ball distance: 0.25 uu, max ball
  distance: 0.25 uu`. Proves the pipeline runs end-to-end without erroring
  — not yet a fidelity measurement (see Blocked/Next).

## In progress

- None.

## Blocked

- `RB-RESEARCH-O002` (binary reverse engineering of the shipped Rocket
  League client) — blocked on two things: (1) explicit owner sign-off after
  a legal/practical review, and (2) practically, this sandboxed environment
  has no access to the Rocket League client binary at all, so any actual RE
  work would have to happen on the owner's own machine. See
  `docs/research/RESEARCH-BACKLOG.md`.
- `RB-VERIFY-001`'s stricter manual single-timestamp cross-check (one ball
  position pinned against a remembered/verified instant, e.g. via in-game
  footage or BakkesMod) — the local `corpus_check` gate (40/40 real owner
  replays, see Completed) already closes the "runs correctly on real owner
  data at scale" half of this criterion; this narrower, precision-focused
  half is still open and needs the owner to do the manual cross-check
  locally, since this sandbox has no way to verify an exact remembered
  timestamp.
- `RB-VERIFY-002-FR-001` (the BakkesMod-side capture plugin) — this
  sandboxed environment has no Rocket League install, no BakkesMod, and no
  Windows to build a BakkesMod SDK plugin on at all (same practical
  blocker as `RB-RESEARCH-O002`). `rb_capture_ingest`'s Rust-side parser is
  implemented and tested against a synthetic fixture, but a real capture
  file — and therefore `RB-VERIFY-002`'s acceptance criteria and
  `PHASE-0-CAPTURE-INGEST`'s exit gate — needs this plugin built and run on
  the owner's own machine.
- `PHASE-0-EXIT`'s exit gate isn't fully met yet: `rb_verify_cli` runs
  end-to-end today, but only against a real replay + a *synthetic* capture
  (see above), and the divergence number it produces isn't a meaningful
  fidelity comparison — `RB-VERIFY-003-FR-002`/`FR-003` (car-state
  scoring, timestamp-tolerant alignment) are still open, and there's no
  Phase 1 candidate physics engine capable of consuming recorded inputs
  and producing a comparable trajectory yet (car bodies aren't
  implemented — `RB-PHYSICS-001-FR-004`).

## Next

1. `RB-VERIFY-002-FR-001` — write, build, and run the BakkesMod-side
   capture plugin against ADR-0005's JSON-Lines format, on the owner's own
   Windows/BakkesMod/game environment (this sandbox can't).
2. `RB-PHYSICS-001-FR-004` — extend `rb_physics_bullet` to box-shaped car
   bodies (general 3x3 inertia, box collision, multi-contact resolution).
3. `RB-VERIFY-003-FR-002`/`FR-003` — car-state scoring and timestamp-
   tolerant alignment, needed before a real candidate-vs-recorded run
   means anything.

## Validation

- `cargo fmt --all -- --check`: pass
- `cargo clippy --workspace --all-targets -- -D warnings`: pass
- `cargo test --workspace`: pass (66 tests: 13 in `rb_domain`, 26 in
  `rb_physics_bullet`, 14 in `rb_replay_ingest` (incl. real-fixture
  integration test), 10 in `rb_capture_ingest` (incl. synthetic-fixture
  test), 3 in `rb_verify_cli` (incl. real end-to-end run), plus doc-tests)
- `cargo run -p rb_replay_ingest --bin corpus_check` (local only, not CI):
  40/40 real owner replays parsed cleanly, 2026-08-28
- `cargo run -p rb_verify_cli --bin rb-verify -- <replay> <capture>`
  (manual, 2026-08-28): `frames compared: 5, mean ball distance: 0.25 uu,
  max ball distance: 0.25 uu` against the real replay fixture + synthetic
  capture fixture.

## Risks and decisions needed

- `RB-RESEARCH-O001` (build vs. integrate physics) — **resolved**, see
  ADR-0004.
- `RB-RESEARCH-O002` (binary reverse engineering) — needs explicit owner
  sign-off after legal/practical review before any work starts, and needs
  the owner's own machine/game install since this sandbox has neither.
  Owner: baileyrd.
- `RB-RESEARCH-O003` (capture tooling scope) — **resolved**, see ADR-0005
  (one-off script, JSON-Lines format).
