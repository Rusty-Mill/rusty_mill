# ADR-0049: Each body's lever arm runs to its own contact point

- Status: Accepted
- Date: 2026-10-04
- Deciders: baileyrd
- Related: RB-PHYSICS-001-FR-130, FR-125, FR-107; ADR-0045, ADR-0048
- Supersedes/Superseded by: —

## Context

After ADR-0048 the largest ball frame in the recordings was
`hitjump.jsonl` 274.217 s (47 uu/s): a car at 2300 uu/s into a resting
ball, its top-front edge 12 uu inside it. The ball left at (0, 2884.8,
917.6) uu/s against (0, 2862.8, 958.7) recorded. The speeds agree
(3027 against 3019); the sim's direction is 0.9 degrees too flat. The error
vector, (+22, −41) in (y, z), is along the car-ball friction direction at
the contact, forward and down (0, 0.46, −0.89), and its size is 46.

Friction level does not matter here (0.5 to 10 give the same hit): the
car-ball friction row sticks. A sticking row's impulse is set by the
effective mass at the contact, and the ball's rotational term scales with
the square of its lever arm.

Bullet's `rel_pos1` and `rel_pos2` run to each body's own contact point:
`positionWorldOnA`, which `btManifoldResult::addContactPoint` sets to
`pointInWorld + normalOnB * depth`, on A's surface, and
`positionWorldOnB`, on B's. The port ran both to `Contact::point`, the
point on B. For 12 uu of penetration the ball's arm was 79 uu, not its
91.25 uu radius: 25% less rotational inverse mass, so 22% more sticking
friction impulse, along (0, 0.46, −0.89).

## Decision

- `Contact::point_on_a` is `point - normal * penetration_depth`.
- `solver::setup_two_body_rows` measures body A's lever arm to it. Body B's
  arm runs to `point` as before. The static (body-versus-world) rows are
  unchanged: their contact point is already the body's own.

Alternatives considered:

- Adjust the ball's inertia or the friction to fit this one frame.
  Rejected: the lever arm is how Bullet defines the row, and it fixes
  every deep car-ball hit at once.

## Consequences

- `hitjump` ball per frame: 274.217 s 47 → 10, 37.400 s 36 → 5, 223.200 s
  30 → 8, 235.267 s 28 → 9, 102.317 s 25 → 7, 260.250 s 18 → 8, 215.942 s
  17 → 6, 95.383 s 16 → 5, 27.625 s 12 → 2. The car's error falls on the
  same frames (274.217 s 6.7 → 0.7).
- `hitjump` one-step ball 0.433 → 0.428, car 0.474 → 0.473; k = 30 ball
  21.11 → 20.96. `test2` 12.267 s 16 → 5, kickoff 5.758 s 17 → 10;
  one-step ball 0.091 → 0.084, k = 30 ball 1.39 → 1.27. `front` and `side`
  unchanged.
- Worst regression: 77.667 s, the wheel-over-ball hit, ball 46 → 58 (car
  8 → 6). The ball's error there was already from something else (ADR-0048
  models the ball as a still surface for the wheels); the lever arm now
  exposes it. Frames over 20 uu/s in `hitjump`: one.
- Car-car contacts, which Bullet treats the same way, move by the same
  rule; no recording has two cars.
