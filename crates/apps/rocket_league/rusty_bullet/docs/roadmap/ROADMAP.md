# Roadmap

Sequential phase gates, not a fixed calendar commitment (part-time project;
rough estimates noted per phase from the original scoping). Each phase's
exit gate is tied to the divergence metric (`RB-VERIFY-003`) where
applicable, per ADR-0002's verification-first ordering.

| Unit | Outcome | Depends on | Specs | Exit gate | Status | Evidence |
|---|---|---|---|---|---|---|
| `PHASE-0-BOOTSTRAP` | Governed repo baseline: charter, architecture, spec tree, ADRs, roadmap, traceability, minimal buildable workspace. | — | all | Docs + workspace present, `cargo fmt/clippy/test` green. | In Progress | This commit |
| `PHASE-0-REPLAY-INGEST` | `rb_replay_ingest` parses real `.replay` files into `PhysicsFrame`s. | `PHASE-0-BOOTSTRAP` | RB-VERIFY-001 | RB-VERIFY-001 acceptance criteria met on ≥1 real replay. | Not Started | — |
| `PHASE-0-CAPTURE-INGEST` | BakkesMod capture tool + `rb_capture_ingest` parse a real offline capture into `PhysicsFrame`s with input attached. | `PHASE-0-BOOTSTRAP` | RB-VERIFY-002 | RB-VERIFY-002 acceptance criteria met on ≥1 real capture. | Not Started | — |
| `PHASE-0-EXIT` | End-to-end verification pipeline: real replay + real capture both score via `rb_verify_cli`. | `PHASE-0-REPLAY-INGEST`, `PHASE-0-CAPTURE-INGEST` | RB-VERIFY-003 | Pipeline runs end-to-end and produces a divergence score on ≥1 real replay and ≥1 real BakkesMod capture. | Not Started | — |
| `PHASE-1-PHYSICS-CORE` | Working car/ball rigid-body physics (build-vs-integrate resolved per RB-RESEARCH-O001, ADR-0003's fidelity target met). Est. 2-4 months part-time. | `PHASE-0-EXIT` | RB-PHYSICS-001 | Candidate engine's divergence score against recorded ground truth reaches a threshold calibrated from the first real candidate run (see RB-VERIFY-003 open questions). | Not Started | — |
| `PHASE-2-DETERMINISM` | Physics core steps identically given identical inputs, run to run. Est. ~1 month part-time. | `PHASE-1-PHYSICS-CORE` | RB-SIM-001 | Repeated-run divergence score is zero; RB-SIM-001 acceptance criteria met. | Not Started | — |
| `PHASE-3-NETCODE` | Server-authoritative simulation with client prediction/rollback, per ADR-0001. Est. 2-3 months part-time. | `PHASE-2-DETERMINISM` | RB-NET-001 | Own client/server pair reproduces the GDC talk's documented reconciliation behavior under simulated network conditions; owner can play an online-style match against a friend. | Not Started | — |
| `PHASE-4-POLISH` | Open-ended: UX, content, tooling, whatever remains once Phase 3 is playable. | `PHASE-3-NETCODE` | (spec'd when reached) | Defined when reached — deliberately not spec'd in detail at bootstrap. | Not Started | — |

## Notes

- Total estimate for Phases 1-3: ~6-9 months, part-time. Not a deadline —
  phases are dependency gates, and Phase 0 must exit before Phase 1 work
  starts (ADR-0002).
- `RB-RESEARCH-O001` (build vs. integrate) is resolved using data from
  `PHASE-0-EXIT`, before `PHASE-1-PHYSICS-CORE` architecture is finalized —
  see `docs/research/RESEARCH-BACKLOG.md`.
- Online-specific lag/reconciliation validation (the project's original
  diagnostic goal) is addressed in `PHASE-3-NETCODE`, validated against the
  GDC talk's documented architecture rather than live official-server
  traffic (no ground-truth source exists for the latter).
