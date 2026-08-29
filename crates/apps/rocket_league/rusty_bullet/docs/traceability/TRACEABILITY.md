# Traceability

Trace both directions: requirements reach evidence, and material code has a
requirement, defect, maintenance policy, or ADR.

| Requirement | Roadmap | Decision/interface | Implementation | Verification | PR/release | State |
|---|---|---|---|---|---|---|
| `RB-VERIFY-003-FR-001` | `PHASE-0-BOOTSTRAP`, `PHASE-0-EXIT` | `rb_domain::divergence::score`, `rb_verify_cli::score_replay_against_capture` | `crates/rb_domain/src/divergence.rs`, `crates/rb_verify_cli/src/lib.rs` | `rb_domain::divergence::tests` + `rb_verify_cli` tests (3) + manual end-to-end run (6 frames matched, mean/max 0.25 uu) | bootstrap + cli-wiring + timestamp-alignment commits | Implemented, Verified (algorithm; CLI wiring mechanical only, not yet a fidelity comparison) |
| `RB-VERIFY-003-NFR-001` | `PHASE-0-BOOTSTRAP` | `rb_domain::divergence::score` | `crates/rb_domain/src/divergence.rs` | `empty_inputs_score_zero_not_nan` | bootstrap commit | Implemented, Verified |
| `RB-VERIFY-003-FR-002` (car-state scoring) | `PHASE-0-EXIT` | `rb_domain::divergence::score`, `Quat::angle_to` | `crates/rb_domain/src/{divergence,state}.rs` | `rb_domain::divergence::tests` (4 car-scoring tests) + `Quat::angle_to` tests (3) + manual end-to-end run (6 car pairs matched) | car-state-scoring commit | Implemented, Verified (algorithm; not yet a fidelity comparison) |
| `RB-VERIFY-003-FR-003` (timestamp alignment) | `PHASE-0-EXIT` | `rb_domain::divergence::score` (`max_timestamp_delta_secs`), `rb_verify_cli::DEFAULT_MAX_TIMESTAMP_DELTA_SECS` | `crates/rb_domain/src/divergence.rs`, `crates/rb_verify_cli/src/lib.rs` | `rb_domain::divergence::tests` (2 alignment-specific: different tick rates, shorter-sequence matching) + manual end-to-end run | timestamp-alignment commit | Implemented, Verified (algorithm; not yet a fidelity comparison) |
| `RB-VERIFY-001-FR-001..003` | `PHASE-0-REPLAY-INGEST` | `rb_domain::port::PhysicsStateSource`, `boxcars`+`subtr-actor` (see Cargo.toml comment) | `crates/rb_replay_ingest/src/{lib,convert}.rs` | `rb_replay_ingest` unit tests (14) + `corpus_check` bin (40/40 real owner replays) | replay-ingestion + corpus-check commits | Implemented, Verified (real fixture + 40-replay owner corpus) |
| `RB-VERIFY-001-NFR-003` (local corpus gate) | `PHASE-0-REPLAY-INGEST` | `AGENTS.md` local corpus convention | `crates/rb_replay_ingest/src/bin/corpus_check.rs` | manual run against `baileyrd/replays` (40/40 clean) | corpus-check commit | Implemented, Verified |
| `RB-VERIFY-001-FR-004` (input attachment) | `PHASE-0-REPLAY-INGEST` | `rb_domain::ControllerInput` (ADR-0005) | `crates/rb_replay_ingest/src/convert.rs` | 4 new `convert::tests` cases | capture-ingestion commit | Implemented, Verified |
| `RB-VERIFY-002-FR-001` (BakkesMod plugin) | `PHASE-0-CAPTURE-INGEST` | ADR-0005 (format design only) | not started | not started | — | Not Started (blocked, no Windows/BakkesMod/game in this environment) |
| `RB-VERIFY-002-FR-002` (capture parsing) | `PHASE-0-CAPTURE-INGEST` | `rb_domain::port::PhysicsStateSource`, ADR-0005 (JSON-Lines format) | `crates/rb_capture_ingest/src/{lib,wire}.rs` | `rb_capture_ingest` unit tests (10, synthetic fixture) | capture-ingestion commit | Implemented, Verified (synthetic fixture only) |
| `RB-VERIFY-002-NFR-001` | `PHASE-0-CAPTURE-INGEST` | `rb_domain::error::IngestError` | `crates/rb_capture_ingest/src/lib.rs` | `malformed_line_reports_malformed_not_a_panic`, `missing_file_reports_io_error_not_a_panic` | capture-ingestion commit | Implemented, Verified |
| `RB-VERIFY-002-NFR-002` (recording overhead) | `PHASE-0-CAPTURE-INGEST` | depends on FR-001 | not started | not started | — | Not Started |
| `RB-PHYSICS-001-FR-001..003` | `PHASE-1-PHYSICS-CORE-V0` | ADR-0003 (fidelity target), ADR-0004 (Bullet3 port) | `crates/rb_physics_bullet` (`integrate`, `collision`, `solver`, `world`) | `rb_physics_bullet` unit tests (81, incl. sphere-path regression coverage) | bullet3-port + car-box + ball-vs-car + box-vs-box + multi-car + driven-input + boost commits | Implemented, Verified (v0 scope; not yet scored against real data) |
| `RB-PHYSICS-001-FR-004` (car boxes, general inertia, multi-contact, ball-vs-car collision) | `PHASE-1-PHYSICS-CORE` | `RigidBody`/`Shape` (body.rs), `Mat3` (mat3.rs), `Quat::conjugate` (rb_domain) | `crates/rb_physics_bullet` (`body`, `mat3`, `collision`, `solver`, `world`) | `rb_physics_bullet` unit tests (32 new: inertia, box-vs-plane contact counts, multi-contact settling, sphere-vs-box contact generation, two-body solver momentum/restitution, end-to-end ball-vs-car bounce) | car-box + ball-vs-car commits | Implemented, Verified (unit tests only; not yet scored against real data) |
| `RB-PHYSICS-001-FR-005` (constant calibration) | `PHASE-1-PHYSICS-CORE` | depends on `PHASE-0-EXIT` | not started | not started | — | Not Started |
| `RB-PHYSICS-001-FR-006` (car-vs-car collision, wired into a multi-car `PhysicsWorld`) | `PHASE-1-PHYSICS-CORE` | `collision::box_vs_box` (15-axis SAT, face-clip/edge-edge manifold); `PhysicsWorld.cars: Vec<RigidBody>` | `crates/rb_physics_bullet` (`collision`, `solver`, `world`) | `rb_physics_bullet` unit tests (4 for `box_vs_box`: no-contact, 4-point flat manifold, argument-order antisymmetry, partial rotated manifold; a generalized `resolve_contacts_between` no-net-rotation test; 3 for multi-car `PhysicsWorld`: builder appends cars, sequential `player_id`s, end-to-end two-cars-bounce-off-each-other) | box-vs-box + multi-car commits | Implemented, Verified (unit tests only, but wired into a real N-car scene — not just detection in isolation; not yet scored against real data) |
| `RB-PHYSICS-001-FR-007` (driven car input — ground throttle/steering) | `PHASE-1-PHYSICS-CORE` | `drive::apply_driven_forces`; `PhysicsWorld.car_inputs`/`set_car_input` | `crates/rb_physics_bullet` (`drive`, `world`) | `rb_physics_bullet` unit tests (8 in `drive`: neutral input is a no-op, throttle accelerates/caps/reverses/is grounded-only, steer is speed-gated and sign-correct; 2 in `world`: end-to-end throttle-driven forward motion + `frame()` input reporting, and a no-input-set regression guard) | driven-input commit | Implemented, Verified (ground throttle/steering; jump/air-control/handbrake not implemented; unit tests only, not yet scored against real data) |
| `RB-PHYSICS-001-FR-008` (boost) | `PHASE-1-PHYSICS-CORE` | `drive::apply_driven_forces` (boost force + drain); `PhysicsWorld.car_boost`/`set_car_boost` | `crates/rb_physics_bullet` (`drive`, `world`) | `rb_physics_bullet` unit tests (4 in `drive`: boost accelerates regardless of ground contact, drains over time and clamps at zero, no effect when empty, still drains at max speed; 2 in `world`: end-to-end airborne boost-driven forward motion + `frame()` boost reporting, and a new-car-starts-full regression guard) | boost commit | Implemented, Verified (unit tests only; jump/air-control/handbrake not implemented; not yet scored against real data) |
| `RB-SIM-001-FR-001..002` | `PHASE-2-DETERMINISM` | — | not started | not started | — | Not Started |
| `RB-NET-001-FR-001` | `PHASE-3-NETCODE` | ADR-0001 (server-authoritative, client-predicted) | not started | not started | — | Not Started |
| `RB-NET-001-FR-002` | `PHASE-3-NETCODE` | — | not started | not started | — | Not Started |

## ADR traceability

| ADR | Decision | Traces to |
|---|---|---|
| ADR-0001 | Server-authoritative simulation with client-side prediction | `RB-NET-001-FR-001`, `PHASE-3-NETCODE` |
| ADR-0002 | Verification pipeline precedes physics implementation | `PHASE-0-*` gating `PHASE-1-PHYSICS-CORE`, all `RB-VERIFY-*` specs |
| ADR-0003 | Target Bullet-derived fidelity, defer engine choice | `RB-PHYSICS-001`, `RB-RESEARCH-O001` |
| ADR-0004 | Resolve build-vs-integrate via direct Bullet3 source port | `RB-PHYSICS-001-FR-001..003`, `PHASE-1-PHYSICS-CORE-V0`, `RB-RESEARCH-O001` |
| ADR-0005 | JSON-Lines capture file format and shared `ControllerInput` schema | `RB-VERIFY-002-FR-001..002`, `RB-VERIFY-001-FR-004`, `RB-RESEARCH-O003` |

## Research traceability

| Research item | Feeds | State |
|---|---|---|
| RB-RESEARCH-S001..S006 (settled) | ADR-0001, ADR-0002, ADR-0003, RB-VERIFY-001/002/003 non-goals | Cited, no further action needed |
| RB-RESEARCH-O001 (build vs. integrate) | `RB-PHYSICS-001`, `PHASE-1-PHYSICS-CORE-V0` | Resolved by ADR-0004 |
| RB-RESEARCH-O002 (binary RE) | `SYSTEM-ARCHITECTURE.md` legal/IP boundary | Open — blocked on owner sign-off, not scheduled |
| RB-RESEARCH-O003 (capture tooling scope) | `RB-VERIFY-002` | Resolved by ADR-0005 |
