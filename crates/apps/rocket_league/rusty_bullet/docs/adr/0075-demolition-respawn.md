# ADR-0075: Demolition respawn, with the spawn point as an input

- Status: Accepted
- Date: 2026-10-08
- Deciders: baileyrd
- Related: `RB-PHYSICS-001-FR-151`, `RB-PHYSICS-001-FR-142` (demolitions), `RB-RESEARCH-O018`, `docs/roadmap/PARITY-PLAN.md` (workstream D)
- Supersedes/Superseded by: none

## Context

A demolished car was removed for good (FR-142). The game brings it back. Twelve
demolitions were recorded (`gen_demolitions.py`, a boosting attacker and a stopped enemy at
twelve places and headings, 700 ticks each) and read with `respawn_report.py`.

## Findings

- The car is out for exactly 3.000 s (360 ticks) in all twelve.
- It returns at one of the ten kickoff spawn points, at rest, facing the point's heading.
  The first frame back shows it at z = 83 with no boost; the next at z = 36 with 33.33 boost,
  falling from rest.
- The point is random: the same tape gave (256, 3840), (256, -3840) and (2048, -2560) in three
  runs, and it follows neither the victim's position nor its team (blue-side and orange-side
  points for the same orange victim).

## Decision

Option: model the delay, the points and the two-frame state exactly, and make the random
pick an input. `respawn.rs` holds the ten points and the constants;
`PhysicsWorld::set_respawn_point(car, Some(i))` (and `Env::set_respawn_point`) chooses the
point of a car's next respawn, `None` takes `respawn::default_pick`, a deterministic
stand-in (mixing of tick and car). `rb-verify`'s recorded-input replay reads the point a
recording shows (`nearest_spawn_point`) and passes it, so a replay can match.

## Consequences

### Positive

- The victim of a recorded demolition follows the game for the whole tape: mean 0.1 to
  0.3 uu over 700 ticks, the same spawn point given.
- Two demolitions are golden fixtures (`demo_4`, `demo_9`).

### Negative / tradeoffs

- Parity of a free run cannot be exact: the game's pick is random, so a policy sees one
  of ten places. Callers that want the game's distribution should pick uniformly.
- A single frame can differ by 47 uu where the game's capture skips the z = 83 frame
  (4 of 12 recordings show only the z = 36 frame).
- Only the first respawn of a car is replayed from a recording; a second uses the default.

## Validation and revisit triggers

- `world` tests: out for 359 ticks, back on the 360th at the chosen point, raw state; the
  next tick z = 36 and a third of a tank with a gravity tick; the default pick is deterministic.
- Revisit if the pick turns out to follow a rule (rotation of unused points, team) with more data.
