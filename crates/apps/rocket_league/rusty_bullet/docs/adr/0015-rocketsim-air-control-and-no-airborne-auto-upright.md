# ADR-0015: RocketSim's air control and damping; no airborne auto-upright

- Status: Accepted
- Date: 2026-10-01
- Deciders: baileyrd
- Related: RB-PHYSICS-001-FR-084, FR-060, FR-068, FR-071, FR-083; ADR-0013,
  ADR-0014
- Supersedes/Superseded by: amends ADR-0009 (air control)

## Context

After FR-083, the real-capture trace for 4.3-5.0 s matched the flip's
vertical stall, but orientation error still grew from 0.15 to ~1.4 rad
between the 4.317 s dodge and 4.97 s. The likeliest source is the spin the
car carries into the flip. Before the dodge the player held roll and pitch
for ~0.08 s, and the port's air control was still a placeholder:

- `AIR_CONTROL_TORQUE = 1e6` as a torque through the car's inertia, with
  only RocketSim's per-axis ratios adopted (FR-068);
- no damping (FR-071's finding);
- an invented airborne auto-upright torque (`LANDING_AUTO_UPRIGHT_TORQUE`),
  which FR-060 found Rocket League does not have.

RocketSim's `_UpdateAirTorque` (`Car.cpp`, fetched 2026-10-01):

- **Torque**: `torque = pitch * -right * 130 + yaw * up * 95 + roll *
  -forward * 400` (`CAR_AIR_CONTROL_TORQUE`).
- **Damping**: `damping = (-right . w) * 30 * (1 - |pitch|) + (up . w) *
  20 * (1 - |yaw|) + (-forward . w) * 50` (`CAR_AIR_CONTROL_DAMPING`),
  applied even with the stick centered.
- **Application**: `applyTorque(I * (torque - damping) * CAR_TORQUE_SCALE)`,
  where `CAR_TORQUE_SCALE = 2 pi / 2^16 * 1000`. This is an angular
  acceleration, independent of the car's inertia.
- **Air throttle**: `THROTTLE_AIR_ACCEL = 200/3` uu/s^2 forward, whether or
  not air control is allowed.

## Decision

Port it directly:

- `air::apply_air_control` adds `(torque - damping) * CAR_TORQUE_SCALE *
  dt` to angular velocity. The flip's gate (ADR-0014) decides whether it
  runs and supplies the pitch lock.
- `air::apply_air_throttle` adds the air throttle.
- The placeholder `AIR_CONTROL_TORQUE`/`YAW_SCALE`/`ROLL_SCALE` constants
  and the auto-upright torque are removed, together with the tests that
  asserted them.

## Consequences

- Air control has no invented magnitudes left. A free-spinning car's
  rotation now decays in the air (roll ~4.8/s), as in Rocket League.
- Behavior change: an airborne car no longer rights itself. Rocket
  League's grounded auto-roll and jump-gated auto-flip remain unmodeled.
- Air throttle now pushes an airborne car forward slightly. One test that
  asserted it had no effect is removed.

## Validation

- Re-run `rb-verify --self-trace test2.jsonl 4.3 5.0`. Orientation error
  through the flip should fall from ~1.4 rad at 5.0 s. Also re-run
  `--self-growth`.

## Amendment (FR-093): air control stays on during a flip

RocketSim turns air control off while a flip's torque lasts, except for the
pitch cancel. The owner's capture disagrees. With `rb-verify --self-trace`'s
car-frame spin (`RB-VERIFY-003` 0.14.0), the 4.317 s diagonal flip (pitch
-1, roll -1) reads, as (roll, pitch) rates:

| | roll -1 held | after release |
|---|---|---|
| recorded | (4.42, 3.27) | settles at (4.10, 3.66) |
| flip torque alone (RocketSim) | (4.17, 3.59) | (4.17, 3.59) |
| flip torque plus air control, pitch locked | (4.40, 3.30) | (4.06, 3.72) |

The held roll stick adds its 400 torque and the roll and pitch dampings act
throughout; the mid-flip yaw input at 4.867 s only nudges yaw (-0.06).
An earlier yaw-only variant (reverted) moved the spin the wrong way.

Decision: in the air, air control always acts; a flip only locks its pitch
torque (`apply_flip_torque` returns the pitch scale, `0` while flipping and
through `FLIP_PITCHLOCK_EXTRA_TIME`). This is the path RocketSim already
takes for a flip cancel.
