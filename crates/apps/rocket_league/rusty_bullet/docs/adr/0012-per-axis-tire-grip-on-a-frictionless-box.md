# ADR-0012: Per-axis tire grip on a box with no floor friction

- Status: Accepted
- Date: 2026-10-01
- Deciders: baileyrd
- Related: RB-PHYSICS-001-FR-081, FR-066, FR-080; ADR-0009, ADR-0011
- Supersedes/Superseded by: partially supersedes ADR-0009 (handbrake as an
  isotropic friction multiplier, and floor grip from the box's own contact
  friction)

## Context

After FR-080 (ADR-0011), the real-capture trace
(`rb-verify --self-trace test2.jsonl 3.7 4.3`) showed the heading
matching, at 0.06 rad error at 4.0 s instead of 0.46. Two problems
remained:

- The direction of travel lagged the heading: about 53° simulated against
  72° recorded at 4.0 s.
- Speed was about 10% low: 906 against 1012 uu/s.

Both have the same cause. The car's box slid on the floor under one
isotropic Coulomb friction. Nothing turned its velocity toward its
heading, and the sliding drag slowed it in every direction.

Real Rocket League cars never slide their box on the floor; four
raycast wheels carry them. RocketSim's wheel code (`Car::_UpdateWheels`,
`btVehicleRL::calcFrictionImpulses`, `RLConst.h`, fetched 2026-10-01)
defines the grip per axis:

- **Sideways**: a damped bilateral impulse,
  `0.2 * rel_vel * jacDiagABInv` per wheel, scaled by
  `LAT_FRICTION_CURVE(slip)` and by `CAR_MASS_BT / 3`.
- **Forward/backward**: the wheels roll freely, except for engine force
  and brake. RocketSim's pedal logic sets a coasting brake (0.15 of full),
  a full brake below 25 uu/s, and a full brake when the throttle opposes
  the car's motion.
- **Handbrake**: ramps in at 5/s and out at 2/s. It scales sideways grip
  by 0.1 and forward/backward grip by 0.5-0.9 (FR-066's anisotropic
  finding), and while held it disables the brake logic.

## Decision drivers

- Every number above is confirmed in RocketSim source. None is tuned.
- The fix must not require the raycast vehicle that ADR-0009 deferred.
- Threading a second, per-direction friction coefficient through the
  solver's row limits touches five call sites (FR-066). Only the floor
  contact needs different treatment.

## Considered options

1. **Frictionless floor contact for the car's box, plus a drive-layer
   tire model** (chosen). The solver's static manifold takes an optional
   friction, and a car's ground manifold passes `None`.
   `drive::ground::apply_ground_control` applies RocketSim's per-axis grip
   to the car's velocity.
2. **Anisotropic friction rows in the solver**: per-direction coefficients
   in the box's contact friction. This still models a sliding box, not
   rolling wheels, and has no place for the rolling and braking logic.
3. **A raycast vehicle with suspension**: the most faithful option, and
   still the long-term direction, but a large subsystem. It also needs the
   hitbox offset, since the real box rides about 18 uu above the floor.

## Decision

Option 1:

- `solver::resolve_manifolds` takes `Option<f32>` static friction, where
  `None` means a frictionless manifold.
- `PhysicsWorld` passes `None` for a car's ground-plane manifold only.
  Walls, curves and every other contact keep their friction.
- `drive::ground` ports the RocketSim pieces above:
  - `pedals` (throttle and brake rules);
  - `slip_ratio`;
  - `tire_grip` (lateral and longitudinal factors with the handbrake
    blend);
  - `lateral_grip_rate`: the sum of the four wheels' side-impulse rates,
    using this port's car inertia at the Octane wheel contact points,
    about 19.5/s for the standard car;
  - `ramp_handbrake`, held in the new `DriveState::handbrake_amount`.
- Boosting counts as full throttle for the pedals, as in RocketSim.
- `HANDBRAKE_FRICTION_MULTIPLIER` and `DriveState::base_friction` are
  removed.

## Consequences

### Positive

- A rolling car goes where it points, and a sliding one recovers at
  RocketSim's rate. A coasting car loses 525 uu/s² rather than about
  325 uu/s² of Coulomb drag in every direction.
- Handbrake drifts keep forward momentum, as in the game.
- `THROTTLE_ACCELERATION` (1600 uu/s²) is now confirmed: RocketSim's
  engine force works out to exactly that over four wheels.
- Braking (3500 uu/s²) now exists. Before, opposing throttle only
  reversed the engine.

### Negative / tradeoffs

- Grip acts on the centre of mass only. Real wheel impulses also roll and
  yaw the car; here yaw comes from ADR-0011's kinematic steering, and
  body roll from cornering is not modeled.
- Slip is computed from the centre-of-mass velocity, not per wheel, so
  front and rear wheels share one slip ratio.
- Only the flat ground plane is tire-carried. Cars on walls, curves or
  the ceiling still slide on box friction, with no sticky force.
- The handbrake ramp's `handbrake_amount` adds per-car state.
- The synthetic capture fixture scores worse (car velocity error 600 to
  868 uu/s), because its car slides sideways at 1000 uu/s while facing
  +X, which tire grip now correctly resists. It was hand-authored, not
  recorded.

## Validation and revisit triggers

- Re-run `rb-verify --self-trace test2.jsonl 3.7 4.3` and `--self-growth`.
  The velocity direction at 4.0 s should approach the recorded 72°, and
  speed should approach 1012 uu/s.
- Revisit when wall or curve driving matters (tire grip on non-floor
  surfaces, sticky force), or if a raycast vehicle is adopted.
