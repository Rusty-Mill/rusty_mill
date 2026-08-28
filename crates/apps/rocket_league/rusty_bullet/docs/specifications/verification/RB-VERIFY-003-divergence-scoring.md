# RB-VERIFY-003 — Divergence Scoring

- Version: 0.2.0
- Status: Draft (core algorithm implemented at bootstrap and wired into
  `rb_verify_cli`; alignment/resampling and car-state scoring are open)
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
`rb_verify_cli` or any future test harness. `rb_verify_cli::score_replay_against_capture`
(in `crates/rb_verify_cli/src/lib.rs`) is the actual composition-root
wiring: it ingests a replay file via `rb_replay_ingest` as the "recorded"
sequence and a capture file via `rb_capture_ingest` as the "candidate"
sequence, then calls `score`. The `rb-verify` binary (`main.rs`) is a thin
argument-parsing/output wrapper over that function, kept separate so the
wiring itself is unit-testable without spawning a process.

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

Unit tests (4, in `rb_domain::divergence`) for the scoring algorithm
itself, plus 3 tests in `rb_verify_cli` exercising the real wiring
(`score_replay_against_capture`) against `rb_replay_ingest`'s vendored
replay fixture and `rb_capture_ingest`'s synthetic capture fixture: a
happy-path run producing a non-empty score, and a missing-file case for
each input reporting `IngestError::Io`. Manually run once
(`cargo run -p rb_verify_cli --bin rb-verify -- <replay> <capture>`,
2026-08-28) against those same two fixtures: `frames compared: 5, mean
ball distance: 0.25 uu, max ball distance: 0.25 uu`. This proves the
ingestion → scoring pipeline runs end-to-end without erroring across both
real adapters — it is **not** a fidelity measurement: the replay and the
capture are unrelated matches (one real, one synthetic) with no physical
reason to resemble each other, so the number itself means nothing yet.
The actual end-to-end run this metric exists for — recorded inputs from
`RB-VERIFY-002` fed into a real Phase 1 candidate physics engine, output
compared against the recorded outcome — still needs that candidate engine
to exist (car bodies, not just sphere-vs-plane) and `RB-VERIFY-002-FR-001`'s
real BakkesMod capture, neither of which exist yet.

## Traceability

See [docs/traceability/TRACEABILITY.md](../../traceability/TRACEABILITY.md).

## Open questions

- FR-002/FR-003 above.
- What divergence threshold counts as "good enough" fidelity for Phase 1 to
  exit — not yet defined; likely needs a first real candidate engine run to
  calibrate against, rather than an arbitrary number chosen in advance.

## Change history

- 0.2.0 (2026-08-28): `rb_verify_cli::score_replay_against_capture` wires
  `score` to the real ingestion adapters (composition root factored out of
  `main.rs` into a testable `lib.rs`). 3 new tests; manually run
  end-to-end against a real replay fixture + synthetic capture fixture,
  proving the pipeline runs without erroring. Explicitly not a fidelity
  measurement yet — see Verification plan.
- 0.1.0 (2026-08-28): Initial draft; core ball-position scoring implemented
  and tested at bootstrap.
