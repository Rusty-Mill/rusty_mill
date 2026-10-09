# ADR-0038: Ball-world penetration is pushed out per point, from RocketSim's 91.25 uu sphere

- Status: Accepted
- Date: 2026-10-04
- Deciders: baileyrd
- Related: RB-PHYSICS-001-FR-118, FR-034, FR-036, FR-108, FR-114; ADR-0027, ADR-0030, ADR-0034
- Supersedes/Superseded by: refines ADR-0027 (no position correction) and ADR-0030 (ERP2 0.2 kept)

## Context

`hitjump.jsonl` 97.892–97.900 s: the ball bounces out of a goal's rounded
back corner from 8.3 uu inside `BALL_RADIUS`. The recorded position moves
5.2 uu further along the contact normal than its velocity carries it in
one tick, and 1.4 uu the tick after from about 4 uu. The simulator moved
it 0: since ADR-0027 the ball's world contacts fold into one combined
contact with no position correction.

RocketSim's solver (`btSequentialImpulseConstraintSolver`, its fork):
- marks every raw ball-world point `m_isSpecial` and skips it in the
  velocity resolve, bouncing the body on one combined contact instead
  (FR-108);
- but `solveGroupCacheFriendlySplitImpulseIterations` has no such skip,
  so each raw point's split-impulse push row still runs, at the arena's
  `m_erp2 = 0.8` and with `m_splitImpulsePenetrationThreshold = 1e30`,
  which makes every contact split;
- the combined contact's own `m_distance1` is the average distance from
  the ball's centre to its contact points, positive, so it pushes
  nothing.

RocketSim's ball is a 91.25 uu sphere (`BALL_COLLISION_RADIUS_SOCCAR`)
that rests at 93.15 (`BALL_REST_Z`) inside its contact band; this port's
`BALL_RADIUS` is 93.15 (FR-036). Penetration for the push is measured
from the 91.25 sphere: 0.8 x (8.3 - 1.9) = 5.1 uu and 0.8 x (4 - 1.9) =
1.7 uu match the recording; 0.8 x 8.3 = 6.6 does not.

## Decision

- `solver::StaticMaterial::PushOnly`: a manifold whose rows take part in
  the split-impulse penetration resolve only, at `ROCKETSIM_ERP2 = 0.8`.
- `PhysicsWorld::step` adds every raw ball-world contact as a push-only
  manifold beside the combined contact, with `penetration_depth` reduced
  by `BALL_RADIUS - BALL_COLLISION_RADIUS` (1.9 uu).
- `body::BALL_COLLISION_RADIUS = 91.25` names RocketSim's sphere. The
  world-contact radius stays `BALL_RADIUS`, so resting height and the
  velocity response are unchanged.
- Car-world rows keep `ERP2 = 0.2` (ADR-0030).

Alternatives considered:
- Push from `BALL_RADIUS` (depth 8.3). Over-pushes by 1.9 uu per contact:
  `hitjump` k = 30 ball 21.54 (was 20.84).
- Make the ball a 91.25 sphere with a 1.9 uu contact band everywhere.
  Equivalent for the ball-world velocity response and the push, but it
  reaches into every world-contact site and FR-114's car radius; deferred
  until something else needs it.

## Consequences

- `test2` k = 30 ball: 0.37 uu (was 0.45). `hitjump` k = 30 ball: 20.88
  (was 20.84); its median, p90 and p99 are unchanged, and the two frames
  that moved most (66.342 s, 244.125 s) are one-tick bounce-timing
  flips at 1,200 uu/s, not a trend. 52 windows improved by more than
  30 uu/s, 43 worsened.
- One-step errors are unchanged on every capture: the push changes
  position only, and one-step re-snaps position each tick.
- `hitjump` 97.908 s stays the worst one-step ball frame (208 uu/s): from
  the recorded 97.900 state the simulator bounces again while the game
  waits one more tick. Bullet's persistent manifold (points kept across
  ticks) is the remaining difference and is not ported.
