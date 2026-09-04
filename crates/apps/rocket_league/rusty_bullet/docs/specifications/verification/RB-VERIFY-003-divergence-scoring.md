# RB-VERIFY-003 — Divergence Scoring

- Version: 0.10.0
- Status: Draft (all four functional requirements implemented and wired
  into `rb_verify_cli`; the first three run end-to-end against a real
  replay AND a real BakkesMod capture, closing `PHASE-0-EXIT`'s own
  literal exit criterion; the fourth, a divergence-growth diagnostic, is
  implemented and sanity-checked against the synthetic capture fixture
  but not yet run against the real capture; open questions remain about
  calibrating an actual "good enough" threshold, see Open questions)
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
- `RB-VERIFY-003-FR-004` (implemented): A divergence-growth diagnostic —
  report how divergence changes *within* a single run, instead of only
  the one whole-run mean/max pair `score` already produces. Needed
  because `RB-PHYSICS-001-FR-077`'s first real candidate-vs-capture run
  produced a whole-run number consistent with near-total trajectory
  divergence, which cannot by itself distinguish gradual compounding
  error (many small modeling gaps adding up) from an abrupt one (a
  specific early mechanic mismatch derailing everything after it) — a
  distinction `RB-PHYSICS-001-FR-005`'s eventual constant calibration
  needs answered first, per that run's own Interpretation note.
  - **Design, as implemented**: `rb_domain::divergence::score_windows(
    recorded: &[PhysicsFrame], candidate: &[PhysicsFrame],
    max_timestamp_delta_secs: f32, window_secs: f32) -> Vec<(f32,
    DivergenceScore)>`, reusing exactly the same nearest-timestamp
    matching `FR-003` already established (both `score` and
    `score_windows` now build on a shared internal `matched_pairs`/
    `score_pairs` pipeline, so the two can never silently drift apart),
    partitioning the matched pairs into consecutive, non-overlapping
    `window_secs`-wide buckets keyed by each pair's own recorded
    timestamp (the first window starts at the first compared pair's
    timestamp, not a fixed `0.0`, so a mid-file seed frame doesn't
    produce a mostly-empty leading window). A run whose pairs all fall in
    one window reproduces exactly the same numbers `score` computes on
    the same input, verified directly by a unit test.
  - **CLI wiring, as implemented**: `rb_verify_cli::score_capture_growth`,
    a sibling entry point sharing the exact seed-frame selection and
    `simulate_recorded` call `score_capture_against_candidate` already
    used (both now call a shared private `seed_and_simulate` helper), and
    a new `rb-verify --self-growth <capture-file> [window-secs]
    [max-timestamp-delta-secs]` mode printing one line per window (window
    start time, frames compared, mean/max ball distance, mean car
    position/rotation/velocity distance) alongside the existing `--self`
    mode.
  - **Default `window_secs`**: `rb_verify_cli::DEFAULT_GROWTH_WINDOW_SECS
    = 1.0` — the one real capture run recorded so far (~23 seconds, 2,818
    frames) prints as roughly 23 rows: small enough to read on one
    screen, fine enough to localize an abrupt derailment to roughly which
    second it started.
  - **Non-goals**: no automatic gradual-vs-abrupt classification
    (changepoint detection, curve fitting) — a human reads the printed
    series and judges its shape, the same "read together" interpretive
    convention `FR-077`'s own Interpretation note already established.
    No new output format (CSV/file export) — stdout table only, matching
    every existing `rb-verify` mode. Does not itself perform
    `RB-PHYSICS-001-FR-005`'s real-data calibration or change any physics
    constant or the seed-frame heuristic — purely a diagnostic read of
    the one real run already recorded, and this FR does not itself run
    it against that real capture (the owner still needs to do that on
    their own machine, the same as `FR-077`'s own run). Only exercises
    the existing single-car freeplay capture; multi-car growth
    diagnostics stay out of scope until a multi-car capture exists, the
    same limit `FR-077` already carries.

## Architecture and interfaces

`rb_domain::divergence::score(recorded: &[PhysicsFrame], candidate:
&[PhysicsFrame], max_timestamp_delta_secs: f32) -> DivergenceScore`. Pure
function, no I/O — callable from `rb_verify_cli` or any future test
harness. `max_timestamp_delta_secs` is a required parameter, not a baked-in
default: what counts as "the same instant" depends on both sequences'
actual sampling rates, which this function has no way to know on its own.
`rb_domain::divergence::score_windows` (`RB-VERIFY-003-FR-004`) has the
same signature plus a `window_secs: f32` parameter and returns `Vec<(f32,
DivergenceScore)>` instead — one score per non-empty time window rather
than one for the whole run. Both functions share a private
`matched_pairs`/`score_pairs` pipeline internally, so `score` is exactly
`score_windows`'s per-window aggregation applied once to every matched
pair.

`rb_verify_cli::score_replay_against_capture` (in
`crates/rb_verify_cli/src/lib.rs`) is the actual composition-root wiring:
it ingests a replay file via `rb_replay_ingest` as the "recorded" sequence
and a capture file via `rb_capture_ingest` as the "candidate" sequence,
then calls `score`. `rb_verify_cli::score_capture_against_candidate` and
`score_capture_growth` share a private `seed_and_simulate` helper (ingest
a capture, find its first grounded/neutral frame, simulate a candidate
forward from there) and differ only in which `rb_domain::divergence`
function they call on the result. The `rb-verify` binary (`main.rs`) is a
thin argument-parsing/output wrapper over these functions (an optional
third CLI argument overrides `rb_verify_cli::DEFAULT_MAX_TIMESTAMP_DELTA_SECS`
in every mode; `--self-growth`'s own second argument overrides
`DEFAULT_GROWTH_WINDOW_SECS`), kept separate so the wiring itself is
unit-testable without spawning a process.

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
- Implemented (divergence-growth diagnostic, FR-004): `score_windows` on
  a run whose pairs all fall within one window reproduces `score`'s own
  numbers on the same input exactly; a synthetic two-window case (first
  window's pairs all identical, second window's pairs all offset by a
  known distance) produces exactly the expected per-window means; the
  first window starts at the first matched pair's own timestamp even
  when earlier recorded frames had no match; a run with no matched pairs
  returns no windows. `rb-verify --self-growth` was run manually against
  the synthetic capture fixture end-to-end without erroring (see
  Verification plan); the real capture run itself is still pending the
  owner's own machine.

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

`RB-VERIFY-003-FR-004`'s divergence-growth diagnostic added 4 unit tests
to `rb_domain::divergence` (14 total) — a single-window run reproducing
`score`'s own numbers exactly, a two-window run with a known offset in
only the second window, a run whose first several recorded frames have no
match (confirming the first window starts at the first *matched* pair's
timestamp, not the first recorded frame's), and a run with no matched
pairs returning no windows — and 3 to `rb_verify_cli` (9 total): a
happy-path run against the synthetic capture fixture, a missing-file case,
and the same no-grounded-neutral-frame case `score_capture_against_candidate`
already covered. Manually run once (`cargo run -p rb_verify_cli --bin
rb-verify -- --self-growth crates/rb_capture_ingest/fixtures/example.capture.jsonl`,
2026-09-04, default `window_secs = 1.0`) against the synthetic capture
fixture: `t=11.78s frames=5 ball mean/max=0.75/2.17 uu car mean
pos/rot/vel=58.75 uu / 0.05 rad / 600.40 uu/s` — a single window, since the
fixture's own 5 frames all fall within one second, proving the CLI mode
runs end-to-end without erroring. This is not yet the diagnostic's real
purpose: running it against `RB-PHYSICS-001-FR-077`'s own real capture
(`test2.jsonl`, ~23 seconds) — the run that would actually show whether
that run's divergence grew gradually or abruptly — still needs the owner
to do that on their own machine, the same as `FR-077`'s own run did.

## Traceability

See [docs/traceability/TRACEABILITY.md](../../traceability/TRACEABILITY.md).

## Open questions

- What divergence threshold counts as "good enough" fidelity for Phase 1 to
  exit — a first real candidate-engine run now exists
  (`RB-PHYSICS-001-FR-077`: `frames compared: 2818, mean ball distance:
  2206.08 uu, ..., mean car position/rotation/velocity distance: 4508.71
  uu / 2.12 rad / 1421.73 uu/s, ...` — see that spec's own Interpretation
  note for the full numbers and reasoning), but it doesn't actually answer
  this question yet: the divergence is consistent with total trajectory
  decorrelation over the run's own ~23-second span, not a bounded gap a
  threshold could meaningfully separate "good" from "bad" against. This
  question stays open until the divergence-growth diagnostic now
  implemented as `RB-VERIFY-003-FR-004` (see Requirements) is actually
  run against that same real capture, giving a number this question can
  actually be answered from. Applies to ball scoring, car scoring, and
  the timestamp-alignment tolerance
  (`rb_verify_cli::DEFAULT_MAX_TIMESTAMP_DELTA_SECS`) alike — all three
  are currently reasoned defaults, not empirically tuned ones.
- Whether `max_timestamp_delta_secs` should ever become adaptive (e.g.
  derived from each sequence's own observed average tick interval) rather
  than a single caller-supplied constant — not needed yet since no real
  candidate engine exists to expose a case where a fixed value is wrong.

## Change history

- 0.10.0 (2026-09-04): `RB-VERIFY-003-FR-004` implemented — a new
  `rb_domain::divergence::score_windows` partitions the same
  nearest-timestamp-matched pairs `score` uses into consecutive
  `window_secs`-wide time buckets and scores each one independently
  (`score` and `score_windows` now share a private `matched_pairs`/
  `score_pairs` pipeline, so they can't silently drift apart); a new
  `rb_verify_cli::score_capture_growth` and `rb-verify --self-growth`
  CLI mode expose it. 4 new `rb_domain::divergence` tests (14 total), 3
  new `rb_verify_cli` tests (9 total). Manually run once against the
  synthetic capture fixture, confirming the CLI mode runs end-to-end; the
  real capture run this diagnostic actually exists for is still pending
  the owner's own machine (see Verification plan).
- 0.9.0 (2026-09-04): Scoped `RB-VERIFY-003-FR-004`, a divergence-growth
  diagnostic — a windowed variant of `score` (`score_windows`) reporting
  divergence within successive time slices of a single run, plus a new
  `rb-verify --self-growth` CLI mode, needed to tell whether `FR-077`'s
  real-run divergence is gradual or abrupt before `RB-PHYSICS-001-FR-005`
  calibrates against it. See Requirements for the full design. Not yet
  implemented — no code change.
- 0.8.0 (2026-09-04): Recorded `RB-PHYSICS-001-FR-077`'s real-capture run
  — this spec's own "good enough" Open Question now has a first real
  number to react to, but the number itself (consistent with near-total
  trajectory divergence) doesn't resolve the question; folded into that
  Open Question's own text rather than kept as a separate bullet. No code
  change.
- 0.7.0 (2026-09-03): Noted that the candidate engine this spec's own
  "good enough" fidelity question depends on is now implemented (not just
  scoped) as `RB-PHYSICS-001-FR-076`/`FR-077`, but still hasn't produced a
  real score — that run remains pending a real capture environment. No
  code change here.
- 0.6.0 (2026-09-02): Noted that the candidate engine this spec's own
  "good enough" fidelity question depends on is now scoped (not
  implemented) as `RB-PHYSICS-001-FR-076`/`FR-077` — see that spec's
  Requirements. No code change.
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
