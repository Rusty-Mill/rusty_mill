# ADR-0047: The ball keeps its contact history across snaps and makes, keeps and folds contacts at Bullet's thresholds

- Status: Accepted
- Date: 2026-10-04
- Deciders: baileyrd
- Related: RB-PHYSICS-001-FR-127, FR-128, FR-121, FR-126; ADR-0041, ADR-0046
- Supersedes/Superseded by: amends ADR-0041 (snapping)

## Context

ADR-0046 left one lead: carrying the ball's mesh manifold through
snaps (what the game does, and what `--self-kstep` does inside a window)
would put the hitjump one-step ball at 0.464 and zero the 97.908 s frame,
but 20.800 s regressed 2 → 109 uu/s.

Tracing that tick: the step from 20.783 s has a full manifold (4 points)
and three new ones (a 28.5° facet point and two on the 39.5° facet). The
port's sort evicted the 28.5° point; the game kept it (the recorded
impulse needs it). The carried set going in differed from the game's
because the port kept points 1.86 uu longer than Bullet does.

Bullet's ball is a 91.25 uu sphere. `SphereTriangleDetector` makes a
contact while the centre is within `radius + threshold` of a triangle and
`validContactDistance` keeps a point while its distance from the surface
is at most `threshold`, where the threshold is `getContactBreakingThreshold`
= 0.02 × 95.25 = 1.905 uu (ADR-0041). The port's ball contact sphere is
93.15 uu (the resting height, 1.9 uu outside Bullet's), so Bullet's
bounds are 0.005 uu past it. The port used 0.01 uu to make a contact and
1.863 uu (0.02 × 93.15) to keep one.

A grid over the variants, carried one-step on hitjump (resets excluded):

| fold | keep | make | ball mean | frames over 50 |
|---|---|---|---|---|
| nearest | 1.863 | 0.01 | 0.464 | 10 |
| nearest | 0.005 | 0.01 | 0.439 | 2 |
| nearest | 0.005 | 0.005 | 0.435 | 1 |
| never (RocketSim) | 0.005 | 0.005 | 0.506 | 15 |

RocketSim's `getCacheEntry` returns -1 (never folds); the captures are
the game's, and folding fits them far better, so the game folds.

## Decision

- `mesh::SphereLimits { admit, retain, breaking }`; `SphereLimits::BALL`
  is 0.005 / 0.005 / 1.905 (`body::BALL_CONTACT_SLACK`,
  `body::BALL_BREAKING_THRESHOLD`). `SphereLimits::plain(radius)` keeps a
  bare sphere's old values for the car path and tests.
- `PhysicsWorld::snap_to_frame` clears the ball's mesh manifolds only if
  the snapped ball is more than `BALL_RADIUS` from where the world left
  it. The manifold is hidden state like the drive state a snap already
  keeps.

Alternatives considered:

- Never fold (RocketSim). Rejected by the table.
- Always clear on snap (FR-121 as written). Kept the 97.908 s frame at 207
  uu/s, an artifact of the missing history.
- Clear on any velocity or contact discontinuity. Not needed: a reset is
  a position jump of thousands of uu, a hit moves the ball by at most a
  frame's travel.

## Consequences

- `hitjump` one-step ball 0.435 (was 0.490, resets excluded); frames over
  50 uu/s 18 → 1. 97.908 s 207 → 0, 97.917 s 103 → 2, 97.933 s 110 → 2,
  73.958 s 104 → 3, 104.392 s 103 → 4, 40.042 s 101 → 2, 39.850 s 102 → 3,
  20.767 s 59 → 4. Worst regression 20.558 s 3 → 14. k = 30 ball 21.18
  (was 21.24).
- `test2`, `front` and `side` unchanged; no car value moves.
- The largest ball frame is now the aerial car hit at 77.667 s (135 uu/s),
  not a contact-shape frame.
- The one-step number now includes the simulated manifold's own history;
  a frame's error is no longer from one cold step alone. A reset inside
  the capture still starts clean.
