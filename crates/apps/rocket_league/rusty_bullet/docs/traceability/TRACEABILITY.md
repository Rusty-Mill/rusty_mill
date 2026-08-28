# Traceability

Trace both directions: requirements reach evidence, and material code has a
requirement, defect, maintenance policy, or ADR.

| Requirement | Roadmap | Decision/interface | Implementation | Verification | PR/release | State |
|---|---|---|---|---|---|---|
| `RB-VERIFY-003-FR-001` | `PHASE-0-BOOTSTRAP` | `rb_domain::divergence::score` | `crates/rb_domain/src/divergence.rs` | `rb_domain::divergence::tests` (4 tests) | bootstrap commit | Implemented, Verified |
| `RB-VERIFY-003-NFR-001` | `PHASE-0-BOOTSTRAP` | `rb_domain::divergence::score` | `crates/rb_domain/src/divergence.rs` | `empty_inputs_score_zero_not_nan` | bootstrap commit | Implemented, Verified |
| `RB-VERIFY-003-FR-002` (car-state scoring) | `PHASE-0-EXIT` | — | not started | not started | — | Not Started |
| `RB-VERIFY-003-FR-003` (timestamp alignment) | `PHASE-0-EXIT` | — | not started | not started | — | Not Started |
| `RB-VERIFY-001-FR-001..004` | `PHASE-0-REPLAY-INGEST` | `rb_domain::port::PhysicsStateSource` | `crates/rb_replay_ingest/src/lib.rs` (stub: `NotImplemented`) | `unimplemented_adapter_reports_not_implemented_not_a_panic` | bootstrap commit | Stubbed, Not Verified against real data |
| `RB-VERIFY-002-FR-001..002` | `PHASE-0-CAPTURE-INGEST` | `rb_domain::port::PhysicsStateSource` | `crates/rb_capture_ingest/src/lib.rs` (stub: `NotImplemented`) | `unimplemented_adapter_reports_not_implemented_not_a_panic` | bootstrap commit | Stubbed, Not Verified against real data |
| `RB-PHYSICS-001-FR-001..003` | `PHASE-1-PHYSICS-CORE-V0` | ADR-0003 (fidelity target), ADR-0004 (Bullet3 port) | `crates/rb_physics_bullet` (`integrate`, `collision`, `solver`, `world`) | `rb_physics_bullet` unit tests (26) | bullet3-port commit | Implemented, Verified (v0 scope; not yet scored against real data) |
| `RB-PHYSICS-001-FR-004` (car boxes, general inertia) | `PHASE-1-PHYSICS-CORE` | — | not started | not started | — | Not Started |
| `RB-PHYSICS-001-FR-005` (constant calibration) | `PHASE-1-PHYSICS-CORE` | depends on `PHASE-0-EXIT` | not started | not started | — | Not Started |
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

## Research traceability

| Research item | Feeds | State |
|---|---|---|
| RB-RESEARCH-S001..S006 (settled) | ADR-0001, ADR-0002, ADR-0003, RB-VERIFY-001/002/003 non-goals | Cited, no further action needed |
| RB-RESEARCH-O001 (build vs. integrate) | `RB-PHYSICS-001`, `PHASE-1-PHYSICS-CORE-V0` | Resolved by ADR-0004 |
| RB-RESEARCH-O002 (binary RE) | `SYSTEM-ARCHITECTURE.md` legal/IP boundary | Open — blocked on owner sign-off, not scheduled |
| RB-RESEARCH-O003 (capture tooling scope) | `RB-VERIFY-002` | Open — resolved when `PHASE-0-CAPTURE-INGEST` starts |
