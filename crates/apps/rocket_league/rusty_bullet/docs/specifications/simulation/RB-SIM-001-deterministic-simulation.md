# RB-SIM-001 — Deterministic Simulation

- Version: 0.1.0
- Status: Draft (placeholder — full design deferred to Phase 2 start)
- Owners: baileyrd
- Depends on: RB-PHYSICS-001
- Supersedes: none

## Purpose and scope

Ensure the Phase 1 physics core steps identically given identical initial
state and inputs, run to run and (eventually) machine to machine — a
prerequisite for Phase 3's client-prediction/rollback netcode, which
requires the client to be able to replay/resimulate ticks and get the same
result the server would.

## Non-goals

Not specifying physics behavior itself (Phase 1's concern) — only the
determinism property layered on top of it. Not addressing cross-platform
floating-point determinism in detail yet (open question, likely to need
fixed-point or careful float-op discipline — see Open Questions).

## Context and terminology

- **Determinism**: same inputs + same initial state ⇒ bit-identical (or at
  least divergence-score-zero) output, across repeated runs.

## Requirements

- `RB-SIM-001-FR-001` (open): Given identical initial state and input
  sequence, two separate simulation runs on the same machine produce
  identical output.
- `RB-SIM-001-FR-002` (open, stretch): The same holds across different
  machines/architectures — needed for Phase 3's server/client parity, but
  may be descoped to "close enough per the divergence metric" if bit-exact
  cross-platform determinism proves impractical.

## Architecture and interfaces

TBD — depends on Phase 1's physics core implementation.

## Data/state and invariants

TBD.

## Errors, failure, recovery, and observability

TBD.

## Security, privacy, and compatibility

TBD.

## Acceptance criteria

TBD — a repeated-run determinism test is the minimum bar; defined fully
once Phase 1 lands.

## Verification plan

Automated repeated-run tests once the physics core exists; scored via
`RB-VERIFY-003` as an additional check (candidate run twice, divergence
between the two runs should be zero).

## Traceability

See [docs/traceability/TRACEABILITY.md](../../traceability/TRACEABILITY.md).

## Open questions

- Floating-point determinism strategy (fixed-point vs. disciplined float
  ops vs. accept-and-measure-drift) — unresolved, deferred to Phase 2.
- Whether determinism needs to hold across CPU architectures/compilers, or
  only same-binary reproducibility is required for this project's actual
  netcode needs.

## Change history

- 0.1.0 (2026-08-28): Placeholder created at bootstrap; full spec deferred
  to Phase 2 start.
