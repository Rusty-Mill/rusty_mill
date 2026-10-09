# ADR-0076: A bump needs a level nose; fast starts are not lagged in the replay scorer

- Status: Accepted
- Date: 2026-10-08
- Deciders: baileyrd
- Related: `RB-PHYSICS-001-FR-152`, FR-140/143 (bumps), ADR-0068, `RB-RESEARCH-O018`, `RB-VERIFY-003-FR-016`
- Supersedes/Superseded by: amends ADR-0068 (bump conditions)

## Context

`bumpf_flip` (a car front-flipping into a stopped one) scored 39 / 66 uu, and the
two-car scorer said "lag 3". Reading the recording at the right alignment showed two
separate things.

1. The scorer's start lag (0 to 3 ticks, chosen by the smallest whole-run error) shifts the
   recording against the port. For a car moving at the start that is a constant offset of
   speed x lag / 120 (25 uu at 1000 uu/s, 38 uu at 2300) in every row, and a whole-run
   minimum can pick a lag to hide a real difference. At lag 0 the flip attacker matched the
   game to 0.1 uu for 36 ticks and the error began exactly at the hit.
2. At the hit the game gave the victim no bump: it accelerated 270, 369, 553 ... to
   770 uu/s over 8 ticks while the attacker fell from 1540 to 690, both ending at half the
   attacker's speed (equal masses, inelastic). The port gave it 1739 uu/s at once. The
   attacker was mid-flip: nose pitched -85 to -77 degrees. A car thrown up by a bump and
   hitting a third (`tri_chain`) was level (-2 to -7 degrees) and did bump: 2253 uu/s.

## Decision

- A bump (and a demolition) needs the bumper's nose within 45 degrees of the horizontal
  (`|forward.z| <= sin 45`, `BUMP_MAX_FORWARD_Z`). The threshold is an assumption between
  7 and 77 degrees, where no recording exists.
- `compare_scenario_recorded` lags a start only by the frames in which the recorded car has
  not moved when the car starts above 300 uu/s (`leading_still_frames`); a car at rest or
  slow is lagged as before.

## Consequences

### Positive

- `bumpf_flip` 202 / 341 uu (honest alignment, old rule) -> 20.6 / 43.8 uu.
- The earlier two-car numbers for fast starts no longer carry a speed-dependent offset.

### Negative / tradeoffs

- The fast-start numbers of earlier notes (`bumpd_*`, `bumpf_flip` 39 / 66) are not comparable.
- The pitch threshold is a guess; a recording of a car at 20 to 70 degrees would settle it.
- The flip still has a 20 uu error: the port's contact is a tick early.

## Validation and revisit triggers

- `world` test: a nose pitched 60 degrees bumps nobody; the bump tests (level noses) unchanged.
- `scenario` test: `leading_still_frames`; the golden gate unchanged.
- Revisit when a flip or an air-roll hit at 20 to 70 degrees is recorded.
