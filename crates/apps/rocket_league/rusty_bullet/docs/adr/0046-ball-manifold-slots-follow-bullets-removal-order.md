# ADR-0046: The ball's mesh manifold drops and folds points in Bullet's slot order

- Status: Accepted
- Date: 2026-10-04
- Deciders: baileyrd
- Related: RB-PHYSICS-001-FR-126, FR-115, FR-116, FR-121; ADR-0035, ADR-0036, ADR-0041
- Supersedes/Superseded by: —

## Context

`hitjump.jsonl` 104.375–104.408 s: the ball grinds up a back-wall fillet,
touching two facets a tick, each facet two triangles. The port finds the
same four candidate points as Bullet would, yet three of the five ticks
were 92–104 uu/s off. Taking every subset of the four points as the
manifold and solving each tick from the recorded state showed what the
game held: at 104.383 s one point on the facet being left and both on the
facet being entered (average normal 24.6°, not the 22.9° of all four), at
104.392 s likewise, at 104.400 s both on the facet being left and one on
the next, at 104.408 s all four. Carrying manifolds across ticks (as
`--self-kstep` does inside a window, and as the game always does) gave
the right three at 104.383 s and then the wrong set twice (191 and 357
uu/s).

Hand-running Bullet's `sortCachedPoints` on the port's own slots showed
why: its area terms pair slots by position (`res1` differences slots 3
and 2, `res2` slots 3 and 1, `res3` slots 2 and 1), so the slot order
decides which point a full manifold gives up. Bullet's
`refreshContactPoints` walks from the last slot down and
`removeContactPoint` moves the last entry into the vacated slot; the port
kept the survivors in order. Bullet's `getCacheEntry` also folds a new
point into the nearest kept one within the threshold; the port took the
first.

## Decision

- `mesh::BallManifold::refresh` drops a slot by `swap_remove`, walking
  from the last slot down.
- `mesh::add_manifold_point` folds into the nearest kept point within the
  threshold.
- A snapped ball still starts with no manifold (FR-121): one-step stays
  the one tick's model error.

Alternatives considered:

- Carrying manifolds through snaps in one-step verification. With this
  fix it would read `hitjump` ball 0.464 (was 0.490), fixing the 97.908 s
  frame (207 → 0) and six others, but 20.800 s regresses (2 → 109): the
  warm chain evicts the 61.5° facet point the game keeps. One rule still
  differs there; left as a lead rather than change what one-step means.
- Bullet's sphere breaking threshold (ADR-0041's 1.905 uu) in place of
  the port's 1.863: no effect on the grind; not changed.

## Consequences

- The carried chain on 104.375–104.408 s reads 4, 3, 3, 2, 2 uu/s.
- `hitjump` k = 30 ball 21.24 (was 21.29): 20.767 s 26 (was 164),
  20.858 s 43 (was 150), 20.917 s 4 (was 98), 20.942 s 4 (was 131),
  20.983 s 1 (was 112), 20.992 s 4 (was 112), 111.308 s 497 (was 687),
  111.333 s 206 (was 320). Car unchanged.
- One-step unchanged by construction; `test2` unchanged in every view.
- The recorded spin changes confirm the port's ball-world friction and
  torque at every grind tick (0.45 vs 0.55 rad/s at 104.383 s), so the
  remaining 20.800 s difference is in the manifold, not the solve.
