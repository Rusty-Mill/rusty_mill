# ADR-0019: Ground-jump force follows the real capture, not RocketSim's pre-min scale

- Status: Accepted
- Date: 2026-10-01
- Deciders: baileyrd
- Related: RB-PHYSICS-001-FR-091, FR-092, FR-064, FR-085, FR-090; ADR-0014,
  ADR-0018
- Supersedes/Superseded by: revises FR-064's `JUMP_PRE_MIN_ACCEL_SCALE`;
  amends ADR-0018 (when the suspension damper reads the car's velocity)

## Context

After FR-090 the earliest real-capture error was the 4.142 s ground jump
(`rb-verify --self-trace test2.jsonl 4.1 4.3`). Per tick, the vertical speed
changed by:

| | press tick | next ticks | after |
|---|---|---|---|
| recorded | 291.67 + 4.0 | +4.0 (6 ticks) | +6.7 |
| candidate | 291.67 + 0.1 | -0.6 (4 ticks) | +6.7 |

The recorded numbers are exact: 6.7 is `JUMP_ACCEL` (1458.3) less gravity
(650) per tick, and 4.0 is that less the sticky force (325) while the wheels
touch. The candidate followed RocketSim:

- `_UpdateJump` scales the force by `JUMP_PRE_MIN_ACCEL_SCALE = 0.62` for
  `JUMP_MIN_TIME`; its source marks this as a TODO ("preferably don't use
  this system at all");
- the port skipped the force on the press tick, which RocketSim applies;
- the suspension damper read the velocity from before the jump impulse, so
  the spring pushed on the press tick (+8 uu/s).

## Decision drivers

- The capture is ground truth; RocketSim is a reverse-engineered
  reference. FR-085 (ADR-0014) already found the real game handles a jump
  press before RocketSim's order.
- Each change is a removal or a reordering; nothing is tuned.

## Considered options

1. **Follow the capture** (chosen): full hold force from the press tick on,
   and the suspension damper reads the car's velocity after the drive
   impulses.
2. **Keep RocketSim's order and scale**: leaves a ~17 uu/s vertical error
   by 4.2 s that every later phase carries.

## Decision

Option 1:

- `JUMP_PRE_MIN_ACCEL_SCALE` is removed; `JUMP_MIN_TIME` still makes the
  hold mandatory.
- `ground_jump` applies the first tick of hold force on the press tick and
  spends that tick of the hold window.
- `drive::wheels` computes each wheel's suspension velocity when the
  suspension is applied, from the car's velocity after this step's drive
  impulses.

## Consequences

### Positive

- A jump from rest gives 295.7 uu/s on the press tick (recorded 295.9),
  then +4.0 per tick while the wheels touch and +6.7 after, as recorded.
- A tapped jump gets the full force through `JUMP_MIN_TIME`, so it rises a
  little higher than before.

### Negative / tradeoffs

- Departs from RocketSim in two places; a RocketSim cross-check would show
  the difference.
- The candidate's wheels still leave the floor 2 ticks before the
  recording's (4 ticks of +4.0 against 6). Its ray geometry is RocketSim's;
  the cause is not yet known.

## Validation and revisit triggers

- `world` test `a_held_ground_jump_gains_speed_as_the_real_capture_does`.
- Re-run `--self-trace test2.jsonl 4.1 4.3`: vertical velocity error by
  4.2 s should fall from ~17 uu/s to a few.
- Real capture, 2026-10-01: confirmed. Vertical speed matches to 0.1 uu/s
  from the press through 4.175 s; 4-5 s growth 7.0 to 5.1 uu.
- Revisit if another capture shows the 0.62 scale, or once the 2-tick
  wheel-contact gap is explained.

## Amendment (FR-092): wheel reach and sticky-force timing

The FR-091 re-trace left one gap: the candidate's wheels left the floor 2
ticks before the recording's. Per step after the 4.142 s jump, with the
origin height at the start of the step:

| start z | recorded grip | recorded vz gain | RocketSim reach |
|---|---|---|---|
| 27.1 | yes | +4.0 | touching |
| 29.7 | yes | +4.1 | beyond (ends 28.0 / 28.8) |
| 32.3 | no | +4.0 | beyond |
| 35.0 | no | +6.7 | beyond |

Two changes fit every recorded tick:

- **Reach without `SUSPENSION_SUBTRACTION`**: rays reach the fully
  extended wheel, `rest + travel + radius` (51.255 / 52.055 uu, origin
  height 30.5 / 31.3), so grip holds from 29.7 and ends before 32.3. The
  subtraction still applies to the pushback reach, which the capture does
  not test.
- **Sticky force one step late**: it acts along the previous step's
  contact normal (`DriveState::sticky_surface_up`), giving the extra +4.0
  in the step from 32.3 with no grip.

The `world` test `a_held_ground_jump_gains_speed_as_the_real_capture_does`
now checks the full pattern: 295.7, six ticks of +4.0, then +6.7.

Real capture, 2026-10-01: confirmed. Every jump tick matches (vx 384.4 vs
385.1 at 4.183 s; vz within 0.1 uu/s); velocity error at 4.2 s 13 to 2.7
uu/s; 4-5 s growth 5.1 to 2.7 uu.

Tradeoff: both are fitted to one jump in one capture. The step delay could
equally be a recording offset; revisit with a second capture that shows a
landing or a jump from a slope.

## Amendment (FR-095): air control waits one step too

The same step after the 4.142 s jump (from origin height 32.3, no grip,
sticky force still on) shows no air control either: the recorded yaw rate
holds at -2.02 rad/s through it and only then changes by the yaw stick's
-0.07 to -0.08 per tick; the candidate applied that air control one step
early, leaving a 0.07 rad/s yaw offset going into the 4.317 s flip.

## Amendment (FR-096): boost already carries the air throttle

`BOOST_ACCEL_AIR` (3175/3) equals the grounded boost (2975/3) plus a full
`THROTTLE_AIR_ACCEL` (200/3): boost forces throttle to 1, and in the air
that throttle's push is part of the boost constant. RocketSim nonetheless
adds the raw stick's air throttle on top. The capture's car, boosting with
throttle -1 over 4.99-5.18 s, gains the full 3175/3 (about 8.7 uu/s per
tick against the candidate's 8.2), so the candidate applies no air
throttle while boost fires (`drive::air_throttle`). The 4.9-5.6 s
re-trace confirms it: the velocity error at the end of that boost (5.175 s)
fell from 17.0 to 8.0 uu/s.

## Amendment (FR-101): jumps push along the car's up axis, tires after

RocketSim's `_UpdateJump` and double jump push along `GetUpDir()`; the
candidate pushed along world up. The car rides pitched ~0.01 rad on its
suspension, so `test2.jsonl`'s 4.142 s jump on throttle gains 2.9 uu/s
forward on its press tick, as an up-axis push does, and `front.jsonl`'s
jump from rest gains ~0.1 uu/s per tick once airborne, the hold force
along the pitched up axis. But that jump from rest keeps under 1 uu/s on
its press tick, where the up-axis push alone gives 3.3: the brake already
cancels it in that tick. RocketSim computes the tire impulses before the
jump; the capture says the tires act after it, so the candidate now jumps
first on the ground.

Real capture, 2026-10-02: confirmed. `test2.jsonl`'s press-tick velocity
error 1.0 uu/s (was 2.6); growth window 4 s 0.64 uu (was 1.27), window 5 s
4.6 uu (was 7.3); `front.jsonl`'s jump from rest within 1.1 uu/s. So the
real game keeps the car "grounded" for one step after its wheels let go,
for both the sticky force and air control (`DriveState::was_on_ground`).
The 4.15-4.35 s re-trace confirms it: the candidate's yaw rate now holds at
-2.03 rad/s through 4.192 s and the rotation error stays 0.00 through the
4.317 s flip.
