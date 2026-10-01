# ADR-0016: Steer through per-wheel side impulses, not a set yaw rate

- Status: Accepted
- Date: 2026-10-01
- Deciders: baileyrd
- Related: RB-PHYSICS-001-FR-086, FR-080, FR-081, FR-085; RB-VERIFY-003
  0.12.0; ADR-0011, ADR-0012
- Supersedes/Superseded by: supersedes ADR-0011; amends ADR-0012
  (sideways grip is now per wheel, not at the centre of mass)

## Context

ADR-0011 set a grounded car's yaw rate to the no-slip bicycle-model value
each tick, and noted that this is an upper bound because tire slip was not
modeled. The real-capture spin trace (`--self-trace` with angular velocity,
RB-VERIFY-003 0.12.0) showed the cost.

After a steer reversal at 4.05 s, the recorded yaw rate ramps from -1.0
rad/s (4.10 s) to -1.7 rad/s (4.14 s), while the candidate snapped to its
-2.17 rad/s target. The ~0.2 rad heading error this left at the 4.317 s
dodge tilted the flip axis ~10 deg, which accounts for the flip's
remaining orientation error (FR-085).

RocketSim's `btVehicleRL::calcFrictionImpulses` turns the car without ever
setting a yaw rate:

- each wheel's axle (the front ones turned by the steer angle) gets a side
  impulse `-0.2 * rel_vel * jacDiagABInv`, scaled by the lateral friction
  and `CAR_MASS_BT / 3 * dt`;
- `rel_vel` is the contact point's velocity along the axle, so it includes
  the car's spin;
- the impulse is applied at the contact point flattened onto the car's
  plane, so it yaws the car without rolling it.

ADR-0012 already used this impulse, summed into a single centre-of-mass
grip rate.

## Considered options

1. **Per-wheel side impulses at the contact points** (chosen). This is
   RocketSim's mechanism, with nothing new to tune. Steering is the
   steered front wheels' impulses, and rear-wheel impulses resist yaw.
2. **A first-order lag on the ADR-0011 yaw rate**: this needs an invented
   time constant, and the ramp would not depend on speed, grip or
   handbrake the way the real one does.

## Decision

Option 1. `drive::ground::wheel_side_impulses` computes each wheel's
impulse from the same pre-impulse state and applies it at the flattened
contact point. Each wheel's slip ratio, and therefore its grip and the
handbrake's reduction, comes from that wheel's own contact velocity.
`ground::steer_angle` gives the front-wheel angle, using the same curves
as before. The kinematic yaw-rate set (`steer_yaw_rate`) and the
centre-of-mass `lateral_grip_rate` are removed. The engine force and
brakes stay at the centre of mass.

## Consequences

- The turn rate builds up and dies away. From 500 uu/s with full steer:
  0.24 rad/s after one tick, 1.1 after 6 ticks, 1.7 after 11, approaching
  the bicycle rate (~2.2) by 0.25 s. This matches the recorded ramp's
  shape (-1.0 at 6 ticks, -1.7 at 11).
- A released turn decays through the rear wheels' grip instead of stopping
  at once (FR-081's zero-steer target is gone).
- Still not modeled:
  - suspension and per-wheel normal load;
  - engine force through the steered front wheels' direction;
  - roll from cornering.
- The 3.7-4.3 s heading match FR-080 achieved (0.06 rad at 4.0 s) needs a
  re-trace: the ramp now lags the target, as the recording does, but the
  magnitude is untested on real data.

## Validation

- Re-run `rb-verify --self-trace test2.jsonl 3.7 4.6`: the recorded and
  simulated yaw spin should ramp together after 3.742 s and 4.05 s, and
  orientation error at the 4.317 s dodge should fall from ~0.22 rad.
  Re-run `--self-growth`.
