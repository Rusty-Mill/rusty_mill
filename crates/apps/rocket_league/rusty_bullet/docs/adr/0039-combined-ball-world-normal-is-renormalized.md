# ADR-0039: The combined ball-world normal is renormalized

- Status: Accepted
- Date: 2026-10-04
- Deciders: baileyrd
- Related: RB-PHYSICS-001-FR-119, FR-108, FR-049; ADR-0027, ADR-0038
- Supersedes/Superseded by: amends ADR-0027 ("not renormalized")

## Context

`hitjump.jsonl` 56.617 s: the ball hits a goal's sloped back at
2,950 uu/s with its contact point sliding up the slope. The game's bounce
is (0, −1608, 757) with spin −1.31 → −2.72 about x: a normal impulse of
4,693 uu/s and a friction impulse of 58 down the slope, which is exactly
what friction opposing the slide on a solid sphere gives (−185 / 3.5 =
−53, Δω = −1.42). The simulator gave (0, −1561, 913) and spin +1.66: the
same normal impulse, but friction of 105 *up* the slope.

ADR-0027 kept the combined normal as RocketSim's `convertContactSpecial`
leaves it, the plain average of the folded points' normals, 0.99 long
here (eight points spread 22 degrees). Bullet's friction direction is
`vel - normal * (normal . vel)`; with a non-unit normal that keeps 2% of
the normal component, so the friction row is 17 degrees off orthogonal
and `dir . delta` picks up 30% of the 4,694 uu/s normal impulse during
the iterations. The friction row then sees a slide of −371 instead of
+185 and pushes the wrong way, clamped only by the friction cone.

The recording shows no such bleed, so whatever the game does, its
effective normal and friction direction are orthogonal.

## Decision

- `combined_ball_world_contact` normalizes the averaged normal. The
  normal impulse is unchanged by this (the row's scaling cancels the
  length); only the friction direction changes.
- FR-108's statement "not renormalized" is withdrawn; the solver's
  `plane_space` fallback still normalizes defensively.

Alternatives considered:
- Make the friction direction orthogonal to the non-unit normal while
  keeping the normal non-unit. Same effect on friction, and the point
  position would still be scaled by the normal's length. Simpler to
  normalize once.
- Keep RocketSim's exact arithmetic. RocketSim is an approximation of
  the game; the recording decides, and here it decides against it.

## Consequences

- `hitjump` 56.625 s: under 2 uu/s (was 163). One-step ball 0.501 (was
  0.513); k = 30 ball 20.41 (was 20.88).
- `test2` one-step ball 0.099 (was 0.109), max 27 (was 32); k = 30 ball
  0.33 (was 0.37).
- `front` and `side` are unchanged.
- The world test `friction_on_a_goal_slope_opposes_the_contact_points_slide`
  pins the recorded bounce and spin.
