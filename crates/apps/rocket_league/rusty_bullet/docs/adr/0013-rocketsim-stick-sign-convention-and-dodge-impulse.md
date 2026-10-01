# ADR-0013: Read pitch and roll in RocketSim's sign convention; real dodge impulse

- Status: Accepted
- Date: 2026-10-01
- Deciders: baileyrd
- Related: RB-PHYSICS-001-FR-082, FR-059, FR-068, FR-072; ADR-0009
- Supersedes/Superseded by: amends ADR-0009 (dodge and air-control
  details)

## Context

The owner's capture (`rb-verify --self-trace test2.jsonl 4.3 5.0`) records
a dodge at 4.317 s with pitch -1 and roll -1, the stick pushed forward and
left, at ~1200 uu/s:

- The recorded car gained ~621 uu/s forward-left.
- The candidate gained ~2,175 uu/s backward.

From there, position error grew to 1,590 uu by 5.0 s, and orientation
error during the air time grew from 0.18 to 3.1 rad while pitch -1 and
roll -1 were held.

RocketSim's `Car.cpp` (fetched 2026-10-01) shows three differences:

- **Dodge direction**: `dodgeDir = (-controls.pitch, controls.yaw +
  controls.roll)`. Rocket League's pitch is negative with the stick
  forward. The port read +pitch as a forward dodge.
- **Dodge spin**: `flipRelTorque = (-dodgeDir.y, dodgeDir.x)`. The port's
  roll spin had the opposite sign.
- **Air control**: the torque axes are `dirPitch_right = -GetRightDir()`
  and `dirRoll_forward = -GetForwardDir()`. The port used +right and
  +forward.

Yaw and ground steering already matched, which the earlier traces confirm.

Separately, FR-059 kept a placeholder `DODGE_SPEED = 1400` instead of
RocketSim's `FLIP_INITIAL_VEL_SCALE = 500`, for lack of real data. The
capture is that data. RocketSim's formula with 500 predicts ~628 uu/s for
this dodge; ~621 was recorded.

## Considered options

1. **Adopt RocketSim's signs at the input boundary, and 500** (chosen).
   `dodge_stick` returns the dodge's forward component as `-pitch`, and air
   control applies pitch about -right and roll about -forward. The dodge's
   roll spin becomes `-side`. The port's internal "forward component
   positive" convention stays as it is.
2. **Negate pitch and roll in the capture adapter**: this hides a physics
   convention in an I/O adapter, and replay input would need the same
   negation. Rejected.

## Decision

Option 1. `DODGE_SPEED` becomes 500, and RocketSim's
`FLIP_BACKWARD_IMPULSE_SCALE_X = 16/15` is added for backward dodges.

## Consequences

- Recorded inputs drive the candidate the way they drove the game. The
  4.317 s dodge now matches RocketSim's formula, and a regression test
  pins it.
- Every test that encoded the old convention now uses Rocket League's own
  signs. Their intent (forward dodge, nose-down spin) is unchanged.
- Not done here:
  - flip vertical damping (`FLIP_Z_DAMP_*`, visible in the capture
    0.15 s after the dodge);
  - the continuous flip torque over `FLIP_TORQUE_TIME` (FR-069);
  - flip pitch lock.

## Validation

- Re-run `rb-verify --self-trace test2.jsonl 4.3 5.0`. The velocity change
  at 4.325 s should be ~(+620, -27), not (-220, -2163), and orientation
  error during 4.3-4.9 s should stay well below 3 rad.
