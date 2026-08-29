# Specification Registry

IDs remain stable. Link superseding artifacts rather than reusing an ID for
a new meaning.

| ID | Title | Version | Design | Implementation | Verification | Depends on | Owner | Location | Evidence |
|---|---|---:|---|---|---|---|---|---|---|
| `RB-VERIFY-001` | Replay Ingestion | 0.4.0 | In Progress | In Progress (FR-001/002/003/004) | Verified (real fixture + 40-replay owner corpus) | — | baileyrd | [spec](verification/RB-VERIFY-001-replay-ingestion.md) | `crates/rb_replay_ingest` tests (14) + `corpus_check` (40/40) |
| `RB-VERIFY-002` | BakkesMod Offline Capture Ingestion | 0.2.0 | In Progress | In Progress (FR-002) | Verified (FR-002, synthetic fixture only) | — | baileyrd | [spec](verification/RB-VERIFY-002-capture-ingestion.md) | `crates/rb_capture_ingest` tests (10) |
| `RB-VERIFY-003` | Divergence Scoring | 0.4.0 | Draft | All FRs implemented (ball + car scoring, timestamp-tolerant alignment, CLI wiring) | Verified (algorithm + mechanical end-to-end run) | RB-VERIFY-001, RB-VERIFY-002 | baileyrd | [spec](verification/RB-VERIFY-003-divergence-scoring.md) | `crates/rb_domain/src/divergence.rs` tests (10) + `angle_to` tests (3) + `rb_verify_cli` tests (3) |
| `RB-PHYSICS-001` | Physics Core Port | 0.9.0 | In Progress | In Progress (sphere + box bodies, ball-vs-car and car-vs-car collision wired into a multi-car `PhysicsWorld`, ground-driving car input, boost, and handbrake; FR-005 open, jump/air-control open) | Verified (unit tests; not yet scored against real data) | RB-VERIFY-003 | baileyrd | [spec](physics/RB-PHYSICS-001-physics-core-port.md) | `crates/rb_physics_bullet` tests (86) |
| `RB-SIM-001` | Deterministic Simulation | 0.1.0 | Draft (placeholder) | Not Started | Not Verified | RB-PHYSICS-001 | baileyrd | [spec](simulation/RB-SIM-001-deterministic-simulation.md) | — |
| `RB-NET-001` | Client Prediction and Rollback Netcode | 0.1.0 | Draft (placeholder) | Not Started | Not Verified | RB-SIM-001 | baileyrd | [spec](netcode/RB-NET-001-client-prediction-rollback.md) | — |

See [docs/roadmap/ROADMAP.md](../roadmap/ROADMAP.md) for how these map to
phases, and [docs/traceability/TRACEABILITY.md](../traceability/TRACEABILITY.md)
for requirement-level traceability to implementation and verification.
