# RB-VERIFY-003 — Divergence Scoring

- Version: 0.5.0
- Status: Draft (all three functional requirements implemented and wired
  into `rb_verify_cli`; now run end-to-end against a real replay AND a real
  BakkesMod capture, closing `PHASE-0-EXIT`'s own literal exit criterion;
  open questions remain about calibrating an actual "good enough"
  threshold, see Open questions)
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
  max ball-position distance across compared frames, the frame count
  compared, and a nested `CarDivergence` (mean/max car position, rotation,
  and velocity distance, plus the car-pair count compared).
- **Recorded trajectory** / **candidate trajectory**: the two
  `Vec<PhysicsFrame>` sequences being compared.

## Requirements

- `RB-VERIFY-003-FR-001` (implemented): Given two frame sequences, compute
  mean and max ball-position distance across the overlapping length.
- `RB-VERIFY-003-FR-002` (implemented): Extend scoring to car position/
  rotation/velocity, not just ball position — needed before this metric
  can validate car-feel fidelity (Phase 1's actual concern), not just
  ball physics. Cars are matched between the two sequences by
  `player_id` within each frame pair (see Data/state and invariants); a
  car present on only one side of a frame pair is skipped for that
  frame, not an error. Rotation distance is the angle between the two
  quaternions (`Quat::angle_to`, radians), not a per-axis Euler
  difference.
- `RB-VERIFY-003-FR-003` (implemented): Timestamp-tolerant alignment
  between sequences sampled at different tick rates or with a start-time
  offset, replacing the original index-pairwise simplification. Each
  recorded frame is matched to the candidate frame nearest it in
  `timestamp_secs` (an `O(recorded.len() + candidate.len())` merge over
  both sequences' existing chronological order, not a binary search per
  frame); a match only counts if the two frames are within a caller-
  supplied `max_timestamp_delta_secs` of each other, so sequences that
  don't actually overlap in time don't get force-matched to whatever's
  nearest-but-still-distant. See Architecture and interfaces.
- `RB-VERIFY-003-NFR-001` (implemented): Scoring an empty or
  mismatched-length pair of sequences never panics or produces `NaN`.

## Architecture and interfaces

`rb_domain::divergence::score(recorded: &[PhysicsFrame], candidate:
&[PhysicsFrame], max_timestamp_delta_secs: f32) -> DivergenceScore`. Pure
function, no I/O — callable from `rb_verify_cli` or any future test
harness. `max_timestamp_delta_secs` is a required parameter, not a baked-in
default: what counts as "the same instant" depends on both sequences'
actual sampling rates, which this function has no way to know on its own.
`rb_verify_cli::score_replay_against_capture` (in
`crates/rb_verify_cli/src/lib.rs`) is the actual composition-root wiring:
it ingests a replay file via `rb_replay_ingest` as the "recorded" sequence
and a capture file via `rb_capture_ingest` as the "candidate" sequence,
then calls `score`. The `rb-verify` binary (`main.rs`) is a thin
argument-parsing/output wrapper over that function (an optional third CLI
argument overrides `rb_verify_cli::DEFAULT_MAX_TIMESTAMP_DELTA_SECS`),
kept separate so the wiring itself is unit-testable without spawning a
process.

## Data/state and invariants

`DivergenceScore.frames_compared` counts recorded frames matched to a
candidate frame within `max_timestamp_delta_secs` — **not**
`min(recorded.len(), candidate.len())` (that was the old index-pairwise
behavior; nearest-timestamp matching can compare every recorded frame
against a much shorter candidate sequence, or compare none at all if the
two sequences' timestamps never come within tolerance of each other).
`mean_ball_distance` is `0.0` (not `NaN`) when `frames_compared == 0`.
`DivergenceScore.cars.pairs_compared`
counts matched car pairs across all compared frames (not frames, and not
`min(total recorded cars, total candidate cars)`): within each compared
frame pair, a recorded car is matched to a candidate car by equal
`player_id`; unmatched cars (present in only one side) contribute nothing.
All three `cars.mean_*` fields are `0.0`, not `NaN`, when
`pairs_compared == 0`.

## Errors, failure, recovery, and observability

No fallible operations — `score` is total over its inputs by construction.

## Security, privacy, and compatibility

None beyond what applies to the frame data itself (see
`RB-VERIFY-001`/`RB-VERIFY-002`).

## Acceptance criteria

- Implemented (ball scoring): identical trajectories score zero; a known
  constant offset scores exactly that offset; empty inputs score zero
  without panicking.
- Implemented (car scoring, FR-002): identical car states score zero
  across position/rotation/velocity; known position/velocity offsets score
  exactly that distance; a known rotation offset scores the correct angle;
  a car present in only one side of a frame pair is skipped, not an error
  (and doesn't panic); empty/no-car inputs score zero, not `NaN`. (All
  covered by unit tests in `rb_domain::divergence::tests` and
  `rb_domain::state::tests` for `Quat::angle_to`.)
- Implemented (timestamp alignment, FR-003): two sequences sampled at
  different, irregular tick rates align by nearest timestamp, not list
  index, and the resulting per-pair distances match hand-computed
  expectations; a shorter candidate sequence can still be matched against
  every recorded frame it's within tolerance of (not capped at
  `min(len, len)`); frames whose nearest available match still exceeds
  `max_timestamp_delta_secs` are skipped, not force-matched. (Covered by
  unit tests in `rb_domain::divergence::tests`.)

## Verification plan

Unit tests (10, in `rb_domain::divergence`) for the scoring algorithm
itself (6 ball/alignment-scoring — including two added for FR-003:
different-tick-rate sequences aligning by nearest timestamp with
hand-computed expected matches, and a shorter sequence still matching
every in-tolerance recorded frame — and 4 car-scoring: identical states,
position/velocity offsets, rotation offset, a car unmatched on one side),
plus 3 tests in `rb_domain::state` for `Quat::angle_to` (identical
rotation, a known quarter-turn angle, and the quaternion double-cover
case), plus 3 tests in `rb_verify_cli` exercising the real wiring
(`score_replay_against_capture`) against `rb_replay_ingest`'s vendored
replay fixture and `rb_capture_ingest`'s synthetic capture fixture: a
happy-path run producing a non-empty score, and a missing-file case for
each input reporting `IngestError::Io`.

Implementing real timestamp alignment surfaced an actual bug in the
synthetic capture fixture: it was originally authored with timestamps
starting at `0.0`, without checking them against the vendored replay
fixture's real timeline — the replay's ball doesn't spawn (and so
produces no `PhysicsFrame`s, per `rb_replay_ingest`'s "frames where the
ball is unavailable are omitted" rule) until roughly **11.78 seconds** in
(kickoff countdown). Under the old index-pairwise comparison this went
unnoticed, since frame 0 was compared to frame 0 regardless of what
timestamp either actually carried — exactly the failure mode FR-003 exists
to catch. The fixture's timestamps were corrected to start at `11.78`
(same relative spacing) so it actually overlaps the replay's real
timeline; see `crates/rb_capture_ingest/fixtures/README.md`.

Manually run once (`cargo run -p rb_verify_cli --bin rb-verify -- <replay>
<capture>`, 2026-08-28, using `rb_verify_cli::DEFAULT_MAX_TIMESTAMP_DELTA_SECS`
= `0.02`) against those same two (now time-aligned) fixtures: `frames
compared: 6, mean ball distance: 0.25 uu, max ball distance: 0.25 uu, car
pairs compared: 6, mean car position/rotation/velocity distance: 2816.42
uu / 2.36 rad / 1307.87 uu/s, max car position/rotation/velocity distance:
2912.18 uu / 2.36 rad / 1627.83 uu/s`. This proves the ingestion → scoring
pipeline runs end-to-end without erroring across both real adapters, with
real timestamp-tolerant alignment now actually engaged (not merely
present but vacuous, as it would have been against the old, disjoint
fixture timestamps) — it is still **not** a fidelity measurement: the
replay and the capture are unrelated matches (one real, one synthetic)
with no physical reason to resemble each other, so the (very large)
car-distance numbers above mean nothing beyond "these are two different
matches," not "car scoring is broken." The actual end-to-end run this
metric exists for — recorded inputs from `RB-VERIFY-002` fed into a real
Phase 1 candidate physics engine, output compared against the recorded
outcome — still needs that candidate engine to exist (car bodies, not
just sphere-vs-plane); `RB-VERIFY-002-FR-001`'s real BakkesMod capture now
exists (see below).

Re-run for real once `RB-VERIFY-002-FR-001`'s BakkesMod capture plugin was
built and used to record a real freeplay session (`cargo run -p
rb_verify_cli -- crates/rb_replay_ingest/fixtures/subtr-actor-sample.replay
<real capture path>`, 2026-09-02, default tolerance): `frames compared:
343, mean ball distance: 3640.81 uu, max ball distance: 6015.71 uu, car
pairs compared: 343, mean car position/rotation/velocity distance: 4714.78
uu / 2.31 rad / 2127.93 uu/s, max car position/rotation/velocity distance:
7721.40 uu / 3.14 rad / 3938.20 uu/s`. This is the first time the pipeline
has run end-to-end on **two genuinely real inputs** — a real vendored
replay and a real BakkesMod recording, not a hand-authored synthetic
capture — closing `PHASE-0-EXIT`'s own literal exit criterion ("produces a
divergence score on ≥1 real replay and ≥1 real BakkesMod capture"). The
numbers themselves remain exactly as meaningless as a fidelity measurement
as the synthetic run above, for the identical reason: the replay and this
capture are two unrelated freeplay sessions with no physical reason to
resemble each other. Closing that separate, harder problem still needs a
Phase 1 candidate engine that consumes the capture's own recorded input
and produces a trajectory to compare against the capture's own recorded
outcome — genuinely out of this phase's scope, not a Phase 0 exit blocker.

## Traceability

See [docs/traceability/TRACEABILITY.md](../../traceability/TRACEABILITY.md).

## Open questions

- What divergence threshold counts as "good enough" fidelity for Phase 1 to
  exit — not yet defined; likely needs a first real candidate engine run to
  calibrate against, rather than an arbitrary number chosen in advance.
  Applies to ball scoring, car scoring, and now the timestamp-alignment
  tolerance (`rb_verify_cli::DEFAULT_MAX_TIMESTAMP_DELTA_SECS`) alike —
  all three are currently reasoned defaults, not empirically tuned ones.
- Whether `max_timestamp_delta_secs` should ever become adaptive (e.g.
  derived from each sequence's own observed average tick interval) rather
  than a single caller-supplied constant — not needed yet since no real
  candidate engine exists to expose a case where a fixed value is wrong.

## Change history

- 0.5.0 (2026-09-02): Re-ran the existing end-to-end pipeline (no code
  change) against a real BakkesMod capture for the first time, now that
  `RB-VERIFY-002-FR-001`'s plugin exists — `frames compared: 343`, real
  numbers on both real inputs, closing `PHASE-0-EXIT`'s own literal exit
  criterion. See Verification plan for the full run and why the score
  itself still isn't a fidelity measurement.
- 0.4.0 (2026-08-28): FR-003 implemented — `score` gains a required
  `max_timestamp_delta_secs` parameter and now aligns frames by nearest
  timestamp (an `O(n+m)` merge over both sequences' existing chronological
  order) instead of list index; a match outside tolerance is skipped, not
  force-matched. `frames_compared`'s meaning changes accordingly (see Data/
  state and invariants). `rb_verify_cli` gains
  `DEFAULT_MAX_TIMESTAMP_DELTA_SECS` and an optional third CLI argument to
  override it. Fixed a real timestamp bug this surfaced in
  `rb_capture_ingest`'s synthetic fixture (see Verification plan). 2 new
  unit tests in `rb_domain::divergence` (10 total); one existing test
  (`mismatched_lengths_compare_only_the_overlap`) was replaced since its
  premise — sequence length alone caps how many frames compare — no
  longer holds under nearest-timestamp matching.
- 0.3.0 (2026-08-28): FR-002 implemented — `DivergenceScore` gains a
  `cars: CarDivergence` field (mean/max position, rotation, velocity
  distance, pairs compared); cars are matched between sequences by
  `player_id` within each frame pair. Added `Quat::angle_to` (rotation
  distance, radians) to `rb_domain::state`. 8 new unit tests (4 in
  `rb_domain::divergence`, 3 for `angle_to`, plus `rb_verify_cli`'s
  output updated to print car stats). Manually re-run end-to-end; see
  Verification plan for the real numbers.
- 0.2.0 (2026-08-28): `rb_verify_cli::score_replay_against_capture` wires
  `score` to the real ingestion adapters (composition root factored out of
  `main.rs` into a testable `lib.rs`). 3 new tests; manually run
  end-to-end against a real replay fixture + synthetic capture fixture,
  proving the pipeline runs without erroring. Explicitly not a fidelity
  measurement yet — see Verification plan.
- 0.1.0 (2026-08-28): Initial draft; core ball-position scoring implemented
  and tested at bootstrap.
