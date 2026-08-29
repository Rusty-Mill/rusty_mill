# Roadmap

Sequential phase gates, not a fixed calendar commitment (part-time project;
rough estimates noted per phase from the original scoping). Each phase's
exit gate is tied to the divergence metric (`RB-VERIFY-003`) where
applicable, per ADR-0002's verification-first ordering.

| Unit | Outcome | Depends on | Specs | Exit gate | Status | Evidence |
|---|---|---|---|---|---|---|
| `PHASE-0-BOOTSTRAP` | Governed repo baseline: charter, architecture, spec tree, ADRs, roadmap, traceability, minimal buildable workspace. | — | all | Docs + workspace present, `cargo fmt/clippy/test` green. | Done | Bootstrap commits on `claude/rocket-league-server-clone-u74q45` |
| `PHASE-0-REPLAY-INGEST` | `rb_replay_ingest` parses real `.replay` files into `PhysicsFrame`s (via `boxcars` + `subtr-actor`). | `PHASE-0-BOOTSTRAP` | RB-VERIFY-001 | RB-VERIFY-001 acceptance criteria met on ≥1 real replay. Met against a vendored third-party fixture (12,029 frames, sane bounds) and validated at scale against 40 of the owner's own real matches via the local `corpus_check` gate (40/40 clean, sane bounds). A stricter manual single-timestamp cross-check remains open (tracked in `RB-VERIFY-001`) but isn't required to close this unit's gate. | Done | `crates/rb_replay_ingest`, 10 tests + `corpus_check` (40/40 real replays) |
| `PHASE-0-CAPTURE-INGEST` | BakkesMod capture tool + `rb_capture_ingest` parse a real offline capture into `PhysicsFrame`s with input attached. | `PHASE-0-BOOTSTRAP` | RB-VERIFY-002 | RB-VERIFY-002 acceptance criteria met on ≥1 real capture. `rb_capture_ingest`'s JSON-Lines parser (ADR-0005) is implemented and tested against a synthetic fixture; the BakkesMod-side plugin that would produce a real capture (FR-001) is not built — blocked on the owner's own Windows/BakkesMod/game environment, same practical constraint as `RB-RESEARCH-O002`. Exit gate not met until a real capture exists. | In Progress | `crates/rb_capture_ingest`, 10 tests (synthetic fixture only) |
| `PHASE-0-EXIT` | End-to-end verification pipeline: real replay + real capture both score via `rb_verify_cli`. | `PHASE-0-REPLAY-INGEST`, `PHASE-0-CAPTURE-INGEST` | RB-VERIFY-003 | Pipeline runs end-to-end and produces a divergence score on ≥1 real replay and ≥1 real BakkesMod capture. The mechanical half is met: `rb_verify_cli::score_replay_against_capture` runs the full ingest-then-score pipeline against a real vendored replay fixture and a capture file without erroring, with ball scoring, car scoring, and real timestamp-tolerant alignment all engaged (6 frames matched by nearest timestamp within tolerance, 6 car pairs matched by `player_id`) — `RB-VERIFY-003`'s three functional requirements are all now implemented. Not fully met: that capture is synthetic, not a real BakkesMod recording (blocked on `PHASE-0-CAPTURE-INGEST`/`RB-VERIFY-002-FR-001`), and the score itself still isn't a meaningful fidelity number (the replay and capture are unrelated matches, and no Phase 1 candidate physics engine exists yet to actually generate a candidate trajectory from recorded inputs). | In Progress | `crates/rb_domain`, 10 divergence tests + 3 `angle_to` tests + `crates/rb_verify_cli`, 3 tests + manual run |
| `PHASE-1-PHYSICS-CORE-V0` | Bullet3-ported rigid-body integration + sequential-impulse solver, scoped to a dynamic sphere (ball) vs. static plane (ground) — per [ADR-0004](../adr/0004-bullet3-source-port-for-physics-core.md), started ahead of `PHASE-0-EXIT` on the strength of Bullet3's direct relevance/licensing. | `PHASE-0-BOOTSTRAP` | RB-PHYSICS-001 | Free-fall/resting/bounce/friction unit tests pass (met); real divergence scoring against `RB-VERIFY-001`/`002` data deferred to `PHASE-0-EXIT` landing. | Done (v0 scope) | `crates/rb_physics_bullet`, 26 tests |
| `PHASE-1-PHYSICS-CORE` | Extend to car (box) rigid bodies, general inertia tensors, multi-contact resolution, ball-vs-car and car-vs-car collision, multi-car `PhysicsWorld` support; calibrate constants against real recorded data. Est. 2-4 months part-time total for Phase 1. | `PHASE-1-PHYSICS-CORE-V0`, `PHASE-0-EXIT` | RB-PHYSICS-001 | Divergence score against recorded ground truth reaches a threshold calibrated from the first real scoring run (see RB-VERIFY-003 open questions). Box bodies, general 3x3 inertia, multi-contact resolution, sphere-vs-box (ball-vs-car), and box-vs-box (car-vs-car) collision are all implemented, unit-tested, and wired into a real N-car `PhysicsWorld` (`RB-PHYSICS-001-FR-004`/`FR-006`, complete). Not met: constant calibration (`FR-005`, needs real scoring data, which needs `PHASE-0-EXIT` to land first) and a combined multi-body solve for 3+ simultaneously-touching bodies (see spec's Non-goals). | In Progress | `crates/rb_physics_bullet`, 39 new tests (65 total) |
| `PHASE-2-DETERMINISM` | Physics core steps identically given identical inputs, run to run. Est. ~1 month part-time. | `PHASE-1-PHYSICS-CORE` | RB-SIM-001 | Repeated-run divergence score is zero; RB-SIM-001 acceptance criteria met. | Not Started | — |
| `PHASE-3-NETCODE` | Server-authoritative simulation with client prediction/rollback, per ADR-0001. Est. 2-3 months part-time. | `PHASE-2-DETERMINISM` | RB-NET-001 | Own client/server pair reproduces the GDC talk's documented reconciliation behavior under simulated network conditions; owner can play an online-style match against a friend. | Not Started | — |
| `PHASE-4-POLISH` | Open-ended: UX, content, tooling, whatever remains once Phase 3 is playable. | `PHASE-3-NETCODE` | (spec'd when reached) | Defined when reached — deliberately not spec'd in detail at bootstrap. | Not Started | — |

## Notes

- Total estimate for Phases 1-3: ~6-9 months, part-time. Not a deadline —
  phases are dependency gates, and Phase 0 must exit before Phase 1 work
  starts (ADR-0002).
- `RB-RESEARCH-O001` (build vs. integrate) is resolved by ADR-0004 (direct
  Bullet3 port) — see `docs/research/RESEARCH-BACKLOG.md`. `PHASE-1-PHYSICS-CORE-V0`
  started ahead of `PHASE-0-EXIT` on that basis; real divergence-score
  validation of the approach still waits on `PHASE-0-EXIT` landing.
- Online-specific lag/reconciliation validation (the project's original
  diagnostic goal) is addressed in `PHASE-3-NETCODE`, validated against the
  GDC talk's documented architecture rather than live official-server
  traffic (no ground-truth source exists for the latter).
