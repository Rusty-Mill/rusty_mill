# ADR-0054: A body's static-contact lever arm runs to its own point

- Status: Accepted
- Date: 2026-10-04
- Deciders: baileyrd
- Related: RB-PHYSICS-001-FR-135, FR-130
- Supersedes/Superseded by: —

## Context

ADR-0049 ran each body's lever arm to its own contact point for two-body
contacts. Static contacts still used the single contact point on the static
shape. A shift along the normal leaves the normal row's lever unchanged, but
changes the friction rows'. At `test2.jsonl` 8.958 s (a corner impact, 5.7
uu deep) the sim's normal impulse and friction direction were right and the
residual was 19 uu/s.

## Decision

- `solver::setup_rows_with_erp` uses `contact.point_on_a() - body.position`.

Alternatives considered: leaving it, since `hitjump` and `front` regress by
0.8% and 2%. Rejected by the owner's call: the rule is Bullet's
(`positionWorldOnA`) and the capture with wall impacts improves 6%.

## Consequences

- `test2` 8.958 s 19 -> 10.4 uu/s; k = 30 car 2.12 -> 1.99. `hitjump` k = 30
  10.52 -> 10.60, `front` 0.49 -> 0.50; `side` unchanged; 9.125 s unmoved.
