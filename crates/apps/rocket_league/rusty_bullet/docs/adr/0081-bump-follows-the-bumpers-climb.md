# ADR-0081: The ground bump follows the bumper's climb; the scorer counts pads swept by the state set

- Status: Accepted
- Date: 2026-10-08
- Deciders: baileyrd
- Related: `RB-PHYSICS-001-FR-159`, FR-140, FR-153, ADR-0080

## Context

The hive duel tapes (`gen_duel_fuzz.py`) include a car that jumps into its mate's path. In
`duel_910` an airborne car (vz 303, speed 940) bumped a grounded one head-on: the game threw
the victim up at 445 uu/s, the port at 205 (the flat heading plus 0.2 of the speed). Trying the
bumper's 3D velocity direction as the heading reproduced it, and improved every tape with an
airborne bumper on a grounded car (`bumpag_*`: 40 -> 8, 110 -> 8.5, 94 -> 19) and the three-car
tapes, but made `bumpo_900`'s second clip worse: the attacker had been spun by the first clip and
its velocity pointed away from its nose. A horizontal heading from the nose, tilted by the
bumper's vertical speed over its speed along the nose, keeps both.

Separately, the state set carries the car over the boost pads between its old and new place and
the game credits them on the next frame (`duel_911`: tank 33 -> 45 at tick 1). The scorer took the
starting tank from the first matching frame and missed it, so cars with room in the tank had
12 fewer boost in the port for the whole tape.

## Decision

- `bump_velocity_of`: ground bumps (any case but two airborne cars) use the tilted heading.
- `compare_scenario_recorded`: a tank jump of more than 5 on the frame after the start frame is
  counted into the starting tank (scorer only).

## Consequences

- Of the 151 bump, triple-car, demolition and duel tapes, 17 improve and 4 get worse by more than
  1 uu (`duel_868` 3251 -> 4604); the airborne-airborne tapes are untouched; the non-duel total falls
  1735 -> 1292 uu. `duel_910` 3319 -> 638; `duel_911` 371 -> 38 (the pad rule).
- A bumper hitting while falling pushes the victim down by its share (not measured on its own).
