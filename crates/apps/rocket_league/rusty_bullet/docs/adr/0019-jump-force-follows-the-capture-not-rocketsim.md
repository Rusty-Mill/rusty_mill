# ADR-0019: Ground-jump force follows the real capture, not RocketSim's pre-min scale

- Status: Accepted
- Date: 2026-10-01
- Deciders: baileyrd
- Related: RB-PHYSICS-001-FR-091, FR-064, FR-085, FR-090; ADR-0014, ADR-0018
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
