# Release Notes

Tracks notable changes to this repo, one entry per merged change against
`main`, reverse chronological. Pre-1.0, no version tags yet — entries are
keyed by the commit/PR that shipped them.

---

## Car-state divergence scoring
**2026-08-28** · [#11](https://github.com/baileyrd/rusty_bullet/pull/11) (merge commit `a1b8a47`)

- **Added:** `rb_domain::divergence::DivergenceScore` gains a `cars:
  CarDivergence` field — mean/max car position distance, rotation distance
  (radians), and velocity distance, plus the number of car pairs compared
  (`RB-VERIFY-003-FR-002`). Cars are matched between the recorded and
  candidate sequences by `player_id` within each frame pair; a car present
  on only one side of a pair is skipped for that frame, not an error.
- **Added:** `Quat::angle_to` (`rb_domain::state`) — the angle between two
  rotations, in radians. Uses an `atan2`-based half-angle formula rather
  than the more obvious `2.0 * dot.acos()`: `acos` is numerically unstable
  exactly where this metric cares most (near-identical rotations, where a
  tiny `f32` rounding difference would otherwise produce a spuriously
  large angle). Handles the quaternion double-cover (`q` and `-q` are the
  same rotation) via the dot product's absolute value.
- **Changed:** `rb-verify`'s output now prints car-pair count and
  position/rotation/velocity stats alongside the existing ball stats.
- **Verified:** 8 new unit tests in `rb_domain` (4 car-scoring cases: 
  identical states, known position/velocity offsets, a known rotation
  offset, a car unmatched on one side; 3 for `angle_to`). Manually re-run
  end-to-end against the same real replay fixture + synthetic capture
  fixture: `car pairs compared: 5, mean car position/rotation/velocity
  distance: 2823.85 uu / 2.36 rad / 1369.44 uu/s`. As before, these
  numbers are not a fidelity signal — the two fixtures are unrelated
  matches — they only confirm car scoring runs correctly end-to-end.
- 8 new unit tests (73 total in the workspace); `cargo fmt --check`,
  `clippy -D warnings`, and `cargo test --workspace` all pass.

## Divergence scoring CLI wiring
**2026-08-28** · [#9](https://github.com/baileyrd/rusty_bullet/pull/9) (merge commit `f10d017`)

- **Added:** `rb_verify_cli::score_replay_against_capture` (new `lib.rs`)
  — the actual composition-root wiring, ingesting a replay via
  `rb_replay_ingest` and a capture via `rb_capture_ingest` and running
  `rb_domain::divergence::score` on the results. `main.rs` is now a thin
  argument-parsing/output wrapper over it, kept separate so the wiring
  itself is unit-testable without spawning a process.
- **Changed:** `rb-verify`'s output is now a small human-readable summary
  (frames compared, mean/max ball distance) instead of a raw `Debug` dump.
- **Verified:** 3 new unit tests against `rb_replay_ingest`'s vendored
  replay fixture and `rb_capture_ingest`'s synthetic capture fixture
  (happy path, missing-replay, missing-capture). Manually run end-to-end:
  `frames compared: 5, mean ball distance: 0.25 uu, max ball distance:
  0.25 uu`. This proves the ingest → score pipeline runs without erroring
  across both real adapters — explicitly **not** a fidelity measurement,
  since the replay and capture are unrelated matches and
  `RB-VERIFY-003-FR-002`/`FR-003` (car-state scoring, timestamp-tolerant
  alignment) are still open.
- 3 new unit tests (66 total in the workspace); `cargo fmt --check`,
  `clippy -D warnings`, and `cargo test --workspace` all pass.

## BakkesMod capture ingestion — JSON-Lines parser + shared input schema
**2026-08-28** · [#7](https://github.com/baileyrd/rusty_bullet/pull/7) (merge commit `dc7e82f`)

- **Added:** `rb_domain::ControllerInput` and `CarState.input:
  Option<ControllerInput>` (ADR-0005) — a shared controller-input schema
  for both ingestion adapters. `throttle`/`steer` are always a number;
  `pitch`/`yaw`/`roll` are `Option<f32>` since only BakkesMod captures can
  ever populate them (a replay's dodge impulse/torque vectors are a
  different kind of quantity, not an analog stick angle). Resolves
  `RB-VERIFY-001-FR-004`, deferred since replay ingestion landed.
- **Changed:** `rb_replay_ingest::convert` now attaches recovered input
  (throttle/steer normalized from replicated bytes, jump/boost/handbrake
  from `subtr_actor`'s boolean flags) to every car it converts. 4 new unit
  tests (14 total in the crate).
- **Added:** `rb_capture_ingest` now really parses capture files
  (`RB-VERIFY-002-FR-002`/`NFR-001`): the capture format is JSON Lines, one
  `{"timestamp_secs", "ball", "cars"}` object per tick (ADR-0005), decoded
  via a new `wire` module (`serde`/`serde_json`, justified in
  `Cargo.toml`) into `rb_domain::PhysicsFrame`s with every car's `input`
  populated. 10 new unit tests, run against a synthetic, hand-authored
  fixture — see `crates/rb_capture_ingest/fixtures/README.md`.
- **Resolved:** `RB-RESEARCH-O003` (BakkesMod tooling scope) — a one-off
  script writing an unversioned format, not a reusable harness, per
  ADR-0005.
- Known limitation stated plainly, mirroring `RB-RESEARCH-O002`'s own
  practical blocker: the BakkesMod-side plugin that would actually write a
  capture file (`RB-VERIFY-002-FR-001`) has not been built — this
  sandboxed environment has no Rocket League, BakkesMod, or Windows
  environment to build or run it in. `PHASE-0-CAPTURE-INGEST`'s exit gate
  (a real capture, cross-checked against BakkesMod's own overlay) stays
  open until the owner builds and runs that plugin on their own machine.
- 14 new unit tests (63 total in the workspace); `cargo fmt --check`,
  `clippy -D warnings`, and `cargo test --workspace` all pass.

## Replay ingestion — local real-corpus validation gate
**2026-08-28** · [#5](https://github.com/baileyrd/rusty_bullet/pull/5) (merge commit `0b2253d`)

- **Added:** `corpus_check`, a local/gitignored-corpus health-check binary
  (`cargo run -p rb_replay_ingest --bin corpus_check [dir]`,
  `RB-VERIFY-001-NFR-003`) — runs the real `boxcars` + `subtr-actor` +
  `convert` pipeline against every `.replay` file in a directory (default
  `replays/` at the workspace root, already `.gitignore`d) and exits
  non-zero on any parse failure. A checkout with no corpus present is a
  deliberate no-op, matching `RLEvalSystem`'s own gitignored-corpus
  convention.
- **Verified:** run once against 40 of the owner's own real match replays
  (`baileyrd/replays`) — 40/40 parsed cleanly, durations 19s-717s, 2-11
  players per match, ball Z consistently within plausible soccar bounds.
  Closes the "runs correctly on real owner data at scale" half of
  `RB-VERIFY-001`'s owner-data acceptance criterion; the stricter manual
  single-timestamp cross-check remains open. Marks `PHASE-0-REPLAY-INGEST`
  Done.
- No new dependencies; no `rb_domain`/`rb_replay_ingest` library code
  changed. The owner's real replay files are never committed — only
  aggregate results (counts, ranges) appear in this repo's docs.

## Replay ingestion — boxcars + subtr-actor
**2026-08-28** · [#3](https://github.com/baileyrd/rusty_bullet/pull/3) (merge commit `93ad0e9`)

- **Added:** `rb_replay_ingest` now really parses `.replay` files
  (`RB-VERIFY-001-FR-001/002/003`): `boxcars` parses the raw replay/network
  stream, `subtr-actor` resolves it into frame-indexed ball/car
  `RigidBody` state, and a new `convert.rs` maps that into
  `rb_domain::PhysicsFrame`. Verified end-to-end against a real vendored
  replay fixture (12,029 frames, ~428s match).
- **Added:** `subtr-actor` as a dependency, justified in
  `Cargo.toml` — avoids hand-rolling `boxcars`' actor-graph resolution
  (net-cache/property-id resolution, quantized rotation decoding), a
  substantial and error-prone parsing layer with an existing,
  permissively-licensed, purpose-built solution.
- **Changed:** `RB-RESEARCH-S004`'s "replay input is lossy/inferred at
  best" finding is revised — `subtr-actor` actually recovers raw
  throttle/steer bytes and boost/jump/dodge/powerslide booleans directly
  from the replay's replicated input actor. Still not wired into
  `rb_domain`'s types (`RB-VERIFY-001-FR-004` stays open pending a schema
  decision made jointly with `RB-VERIFY-002`).
- Known limitation stated plainly: the vendored fixture is a third
  party's replay, used only to prove the pipeline runs correctly on real
  bytes — it does not satisfy `RB-VERIFY-001`'s acceptance criterion of a
  manually-verified position check against the owner's own match, since
  this environment has no access to the owner's replay files.
- 10 new unit tests (51 total in the workspace); `cargo fmt --check`,
  `clippy -D warnings`, and `cargo test --workspace` all pass.

## Physics core v0 — Bullet3 port (sphere vs. ground)
**2026-08-28** · [#1](https://github.com/baileyrd/rusty_bullet/pull/1) (merge commit `7bdc3fc`)

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
**2026-08-28** · landed directly on `main` at commit `5be2078` (predates this repo's "always PR" convention; no PR exists for it)

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
