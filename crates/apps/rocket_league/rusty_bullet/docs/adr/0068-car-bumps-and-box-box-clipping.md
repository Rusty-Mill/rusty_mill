# ADR-0068: Car bumps, and box-box contacts as `dBoxBox`

- Status: Accepted (validated against the game on 2026-10-07)
- Date: 2026-10-07
- Deciders: baileyrd
- Related: `RB-PHYSICS-001-FR-140`, `FR-141`, `FR-142`, `FR-006`, ADR-0066, ADR-0067,
  RB-RESEARCH-O018
- Supersedes/Superseded by: none (the "bumps are not modeled" note in
  `world.rs` is replaced)

## Context

The two-car recordings of ADR-0067 gave the first car-vs-car ground truth.
The port treated a hit between cars as a plain collision, and was off by 100 to
430 uu on every bump: a car at 500 uu/s pushing a stopped one sent it to 1293
uu/s and into the air in the game, to 496 uu/s in the port.

The same recordings, with the second car on the other team, showed the
demolition (FR-142): a supersonic nose on an enemy removes it for three
seconds.

## Decision drivers

- Match the recorded response across speeds, angles and car roles.
- One mechanism per observed effect, no constants fitted to one recording.
- Keep the one-car recordings exactly as they are.

## Considered options

1. A uniform restitution/friction change for car-car contacts.
2. RocketSim's bump: an extra velocity on the bumped car.
3. As 2, plus fixing the box-box contact generator where the fast hits still
   differ.

## Decision

Option 3. The bump (FR-140) is RocketSim's mechanism with the constants read
off the recordings (they match RocketSim's curve to 1%); the contact fix
(FR-141) came from the failing fast hits: a clipped incident face and
`dBoxBox`'s edge-axis fudge factor.

## Consequences

### Positive

- Every bump scenario within about 10 uu per car (rear 300 to 2100 uu/s, side,
  head-on, off-centre, retreating target); nine are in the golden gate.
- The contact fix removes a spurious pitching torque from every car-car hit.

### Negative / tradeoffs

- A demolished car stays out for good: the game respawns it three seconds
  later at a spawn point it picks, which is not modelled. The bump's air
  variant (a victim in the air) and a bumper on a wall are unmeasured.
- Both changes assume the contact points of one tick; the persistent manifold
  Bullet keeps between ticks is not ported for car-car contacts.

## Validation and revisit triggers

- The `world` and `collision` tests, and the nine bump fixtures in the golden
  gate (`golden_captures.rs`).
- Revisit if a hit with a car in the air or on a wall disagrees, or a policy
  needs the respawn.
