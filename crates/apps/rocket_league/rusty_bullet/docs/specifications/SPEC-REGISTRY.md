# Specification Registry

IDs remain stable. Link superseding artifacts rather than reusing an ID for
a new meaning.

| ID | Title | Version | Design | Implementation | Verification | Depends on | Owner | Location | Evidence |
|---|---|---:|---|---|---|---|---|---|---|
| `RB-VERIFY-001` | Replay Ingestion | 0.2.0 | In Progress | In Progress (FR-001/002/003) | Verified (real fixture, unit tests) | — | baileyrd | [spec](verification/RB-VERIFY-001-replay-ingestion.md) | `crates/rb_replay_ingest` tests (10) |
| `RB-VERIFY-002` | BakkesMod Offline Capture Ingestion | 0.1.0 | Draft | Not Started | Not Verified | — | baileyrd | [spec](verification/RB-VERIFY-002-capture-ingestion.md) | — |
| `RB-VERIFY-003` | Divergence Scoring | 0.1.0 | Draft | In Progress (core algorithm) | Verified (core algorithm, unit tests) | RB-VERIFY-001, RB-VERIFY-002 | baileyrd | [spec](verification/RB-VERIFY-003-divergence-scoring.md) | `crates/rb_domain/src/divergence.rs` tests |
| `RB-PHYSICS-001` | Physics Core Port | 0.2.0 | In Progress | In Progress (v0: sphere-vs-plane) | Verified (v0, unit tests) | RB-VERIFY-003 | baileyrd | [spec](physics/RB-PHYSICS-001-physics-core-port.md) | `crates/rb_physics_bullet` tests (26) |
| `RB-SIM-001` | Deterministic Simulation | 0.1.0 | Draft (placeholder) | Not Started | Not Verified | RB-PHYSICS-001 | baileyrd | [spec](simulation/RB-SIM-001-deterministic-simulation.md) | — |
| `RB-NET-001` | Client Prediction and Rollback Netcode | 0.1.0 | Draft (placeholder) | Not Started | Not Verified | RB-SIM-001 | baileyrd | [spec](netcode/RB-NET-001-client-prediction-rollback.md) | — |

See [docs/roadmap/ROADMAP.md](../roadmap/ROADMAP.md) for how these map to
phases, and [docs/traceability/TRACEABILITY.md](../traceability/TRACEABILITY.md)
for requirement-level traceability to implementation and verification.
