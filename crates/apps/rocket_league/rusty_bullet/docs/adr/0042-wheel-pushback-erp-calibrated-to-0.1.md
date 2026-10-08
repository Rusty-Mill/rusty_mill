# ADR-0042: The wheel pushback's ERP is 0.1, calibrated from a recorded landing

- Status: Accepted
- Date: 2026-10-04
- Deciders: baileyrd
- Related: RB-PHYSICS-001-FR-122, FR-090, FR-091, FR-092; ADR-0034
- Supersedes/Superseded by: —

## Context

`test2.jsonl` 18.358–18.400 s: the car lands nose-first at 231 uu/s down
with its suspension bottomed out (origin height 8.8–10.9 uu against a
rest height of 17) and rebounds over five ticks (+168, +87, +40, +15, +2
uu/s). The simulator, started from each recorded tick, gained 24–27 uu/s
more than the game every tick: the largest car-side error in that
recording after the wall ride, and the same offset showed at 14.38 s.

`btVehicleRL::rayCast` runs Bullet's `resolveSingleCollision` for a
wheel whose ray is shorter than `rest + radius - 0.05 BT`, and
`updateSuspension` adds a quarter of that impulse per wheel as
`m_extraPushback`. Its positional term is `m_erp * -distance / dt` with
Bullet's default `m_erp = 0.2`, which FR-090 ported as `PUSHBACK_ERP`.
Probing the 18.358 s tick: of the simulator's 195 uu/s rebound, 117 came
from the four pushbacks, 48 from the dampers and 37 from the springs.
Sweeping the ERP against `test2` (one-step car mean):

| ERP | 0 | 0.08 | 0.10 | 0.12 | 0.15 | 0.20 |
|---|---|---|---|---|---|---|
| car mean (uu/s) | 0.738 | 0.514 | 0.470 | 0.493 | 0.624 | 0.895 |
| 18.367–18.400 s | 23/22/13/5/1 | 6/6/4/3/1 | 1.8/1.7/1.6/1.2/1.2 | 6/5/5/5/4 | 14/13/12/12/10 | 25/27/27/24/21 |

## Decision

- `drive::wheels::PUSHBACK_ERP = 0.1`. A calibration, not a sourced
  constant: no Bullet or RocketSim constant on this path is 0.1
  (`m_splitImpulseTurnErp` is, but it is not used here). An ERP of 0.2
  over half the penetration would read the same; the recording cannot
  tell them apart, and the ERP is the single knob.
- The velocity term and the quarter-per-wheel split are unchanged.

Alternatives considered:
- Keep 0.2 as the sourced default. The game disagrees at every bottomed-
  out landing in the recordings.
- Drop the positional term (ERP 0). Undershoots: 0.738.

## Consequences

- `test2` one-step car 0.470 (was 0.895), 116 frames better by more than
  3 uu/s and none worse; k = 30 car 5.65 uu/s (was 5.76), position 0.65
  (was 0.68). `front` 0.178 (was 0.192), `side` 0.192 (was 0.219),
  `hitjump` 0.494 (was 0.574). Ball errors unchanged.
- The world test `a_bottomed_out_landing_rebounds_as_recorded` pins the
  18.358 s tick to within 5 uu/s.
- A recording with a harder landing (deeper than the 12 uu travel) would
  test the ERP further from the band this one covers.
