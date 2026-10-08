# ADR-0029: One friction row per contact, as Bullet's default solver mode

- Status: Accepted
- Date: 2026-10-02
- Deciders: baileyrd
- Related: RB-PHYSICS-001-FR-110, FR-049, FR-035, FR-107; ADR-0026
- Supersedes/Superseded by: amends FR-049's second friction direction

## Context

After ADR-0028 the worst one-step ball errors in `test2.jsonl` were the
car hits at 5.758 s (kickoff, 129 uu/s) and 12.267 s (90). Our extra
ball-hit velocity at the kickoff matched RocketSim's formula exactly,
(545, 1095, 315). So the difference was in the solve: we gave the ball
(997, 1167, 484) and its spin z -3.27, while the recording implies
(1057, 1054, 475) and spin z -0.49.

FR-049 gave every contact two friction rows: one along the slip and one
along `slip x normal`. Bullet adds that second row only under
`SOLVER_USE_2_FRICTION_DIRECTIONS`. The default `btContactSolverInfo`
mode, which RocketSim keeps, is `SOLVER_USE_WARMSTARTING | SOLVER_SIMD`
(`convertContactInner`: "By default, each contact has only a single
friction direction"). With friction 2 (car-ball), the second row was
cancelling sideways slip that Bullet leaves alone.

## Decision

- `solver::setup_rows` and `setup_two_body_rows` build two rows: normal
  and one friction row along `friction_direction` (the slip direction, or
  `plane_space`'s first axis with no slip).
- `ContactCache` warm-starts `[normal, friction]`.
- No configuration flag: no RocketSim mode uses two directions.

## Consequences

`--self-onestep` (uu/s):

| Measure | Before | After |
|---|---|---|
| `test2` car mean | 1.72 | 1.59 |
| `test2` 8.958 s corner impact (car) | 173 | out of the top steps |
| `test2` ball mean | 0.17 | 0.11 |
| `test2` 5.758 s ball | 129 | 23 |
| `test2` 12.267 s ball | 90 | 27 |
| `front` car mean | 0.302 | 0.192 |
| `front` 25.967 s nose landing | 156 | out of the top steps (worst now 8) |

Free-running `--self` on `front.jsonl`: mean car position error 34 uu
(was 1026), rotation 0.03 rad (was 0.66).

The 8.958 s corner impact and the 25.967 s nose landing, both open since
FR-104/FR-105, were this second friction row.

The net's symmetric-catch test now leaves ~0.08 uu/s sideways (was 0.016,
still under the sequential loop's 0.25); its bound is 0.15.
