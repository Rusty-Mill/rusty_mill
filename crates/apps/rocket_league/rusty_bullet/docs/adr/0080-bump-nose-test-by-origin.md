# ADR-0080: The bump's nose test uses the other car's origin; the second hive car's input leads a tick

- Status: Accepted
- Date: 2026-10-08
- Deciders: baileyrd
- Related: `RB-PHYSICS-001-FR-158`, FR-140, FR-152, ADR-0068, ADR-0076

## Context

`gen_duel_fuzz.py` records two same-team cars driving at each other (the hivemind plays both
tapes) with a sideways offset, then random tapes. Two findings.

1. The second car's recorded input leads the first's by one tick: with the same offset for both
   cars the port's second car was 6 to 9 uu off after a boosted straight run; reading its input
   one tick later (`HIVE_INPUT_LEAD`) gives 0.1 to 0.4 uu. Only the scorer changes.
2. `duel_865`: a head-on at about 1000 uu/s each, 70 uu off line. The game bumped both cars (each
   up 202 uu/s and back); the port bumped one, because the contact point on the other car lay 26
   uu along its forward axis (the first car's nose hit its front corner), under the 64.5 uu nose
   test of FR-140.

## Decision

A car bumps (or demolishes) when the other car's origin is more than `BUMP_MIN_FORWARD_DIST` ahead
of its own along its forward axis; the contact point is no longer consulted. The 124 other bump,
triple-car and demolition tapes score identically under both rules.

## Consequences

- `duel_865` car 0 error 679 -> 167 uu, `duel_848` 524 -> 447, `duel_875` 1748 -> 1522.
- A side hit by a car's nose still counts for the car whose nose it is; a car hit in its own flank
  by another's nose does not bump the other, as before (its origin is not ahead).
- The level-nose gate (FR-152) and the cooldown are unchanged.
