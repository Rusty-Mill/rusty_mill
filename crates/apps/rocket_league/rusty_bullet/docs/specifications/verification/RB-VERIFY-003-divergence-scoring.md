# RB-VERIFY-003 — Divergence Scoring

- Version: 0.1.0
- Status: Draft (core algorithm implemented at bootstrap; alignment/
  resampling and car-state scoring are open)
- Owners: baileyrd
- Depends on: RB-VERIFY-001, RB-VERIFY-002
- Supersedes: none

## Purpose and scope

Define and compute the single accuracy metric the whole project tunes
against: how far a candidate physics/simulation implementation's output
trajectory diverges from a recorded ground-truth trajectory, given matching
initial state and (where available) matching recorded inputs.

In scope: the scoring algorithm itself (`rb_domain::divergence`), operating
on two `Vec<PhysicsFrame>` sequences regardless of which adapter produced
them.

## Non-goals

- Not responsible for producing either input sequence (`RB-VERIFY-001`,
  `RB-VERIFY-002`) or for running a candidate physics engine to generate
  its output — that's `RB-PHYSICS-001`'s composition-root responsibility
  once it exists.
- Not a validator of online-specific netcode behavior (lag,
  reconciliation). That is validated separately against the GDC 2018 talk's
  documented architecture, since no ground-truth online-input data source
  exists — see [docs/research/RESEARCH-BACKLOG.md](../../research/RESEARCH-BACKLOG.md).

## Context and terminology

- **Divergence score**: `rb_domain::divergence::DivergenceScore` — mean and
  max ball-position distance across compared frames, plus the frame count
  compared.
- **Recorded trajectory** / **candidate trajectory**: the two
  `Vec<PhysicsFrame>` sequences being compared.

## Requirements

- `RB-VERIFY-003-FR-001` (implemented): Given two frame sequences, compute
  mean and max ball-position distance across the overlapping length.
- `RB-VERIFY-003-FR-002` (open): Extend scoring to car position/rotation/
  velocity, not just ball position — needed before this metric can validate
  car-feel fidelity (Phase 1's actual concern), not just ball physics.
- `RB-VERIFY-003-FR-003` (open): Timestamp-tolerant alignment between
  sequences sampled at different tick rates or with a start-time offset,
  rather than the current index-pairwise simplification (see
  `rb_domain::divergence`'s doc comment on why that simplification was
  acceptable for bootstrap but isn't the final answer).
- `RB-VERIFY-003-NFR-001` (implemented): Scoring an empty or
  mismatched-length pair of sequences never panics or produces `NaN`.

## Architecture and interfaces

`rb_domain::divergence::score(recorded: &[PhysicsFrame], candidate:
&[PhysicsFrame]) -> DivergenceScore`. Pure function, no I/O — callable from
`rb_verify_cli` or any future test harness.

## Data/state and invariants

`DivergenceScore.frames_compared` is always
`min(recorded.len(), candidate.len())`; `mean_ball_distance` is `0.0` (not
`NaN`) when `frames_compared == 0`.

## Errors, failure, recovery, and observability

No fallible operations — `score` is total over its inputs by construction.

## Security, privacy, and compatibility

None beyond what applies to the frame data itself (see
`RB-VERIFY-001`/`RB-VERIFY-002`).

## Acceptance criteria

- Implemented: identical trajectories score zero; a known constant offset
  scores exactly that offset; mismatched lengths compare only the overlap;
  empty inputs score zero without panicking. (All four covered by unit
  tests in `rb_domain::divergence::tests`.)
- Open: car-state divergence and timestamp-tolerant alignment each get
  their own acceptance criteria once designed (FR-002, FR-003).

## Verification plan

Unit tests (existing, in `rb_domain`) plus, once a Phase 1 candidate
physics engine exists, an end-to-end run: recorded inputs from
`RB-VERIFY-002` fed into the candidate engine, output compared against the
recorded outcome via this scorer. That end-to-end run is Phase 0's overall
exit criterion — see [docs/roadmap/ROADMAP.md](../../roadmap/ROADMAP.md).

## Traceability

See [docs/traceability/TRACEABILITY.md](../../traceability/TRACEABILITY.md).

## Open questions

- FR-002/FR-003 above.
- What divergence threshold counts as "good enough" fidelity for Phase 1 to
  exit — not yet defined; likely needs a first real candidate engine run to
  calibrate against, rather than an arbitrary number chosen in advance.

## Change history

- 0.1.0 (2026-08-28): Initial draft; core ball-position scoring implemented
  and tested at bootstrap.
