# ADR-0014: A dodge's flip as a timed torque, with vertical damping and pitch-stick cancel

- Status: Accepted
- Date: 2026-10-01
- Deciders: baileyrd
- Related: RB-PHYSICS-001-FR-083, FR-069, FR-070, FR-082; ADR-0013
- Supersedes/Superseded by: amends ADR-0009 (dodge spin), ADR-0013

## Context

After FR-082, the real-capture trace for 4.3-5.0 s matched the 4.317 s
dodge's impulse, but two things still diverged:

- **Vertical speed**: the recorded vz fell from 211 to about -15 uu/s from
  ~0.15 s after the dodge and held there; the candidate fell to -211 uu/s
  by 5.0 s.
- **Orientation**: error grew to ~1.6 rad through the flip.

The port's dodge spin was a single instant `DODGE_ANGULAR_SPEED` kick, and
its flip cancel was a second jump press that zeroed all spin. FR-069 and
FR-070 had already found both differ from Rocket League but deferred the
fix because it needs per-car flip-time state.

RocketSim (`Car.cpp`, `_UpdateAirTorque` and `_UpdateDoubleJumpOrFlip`,
`RLConst.h`) does the following:

- **Flip state**: a dodge stores `flipRelTorque = (-dodgeDir.y,
  dodgeDir.x)` and starts `flipTime`.
- **Torque**: while `flipTime < FLIP_TORQUE_TIME` (0.65 s), `flipRelTorque
  * (FLIP_TORQUE_X 260, FLIP_TORQUE_Y 224)` is applied through the car's
  inertia, as an angular acceleration in the car frame.
- **Air control**: off during the flip. Air-control pitch stays locked
  until `FLIP_TORQUE_TIME + FLIP_PITCHLOCK_EXTRA_TIME` (0.95 s).
- **Cancel**: holding pitch with the same sign as `flipRelTorque.y`
  (against the flip) scales the pitch torque by `1 - |pitch|` and re-enables
  air control.
- **Vertical damping**: from 0.15 s to 0.65 s, vz is scaled by `1 - 0.35`
  per 120 Hz tick while falling, or unconditionally before 0.21 s.
- **Ordering**: torque from the current state comes first, then the jump
  press, then the clock advances and the damping applies. The torque
  therefore starts on the tick after the press. (Amended by
  `RB-PHYSICS-001-FR-085`: the owner's capture shows the real game's spin
  jumping by one tick of flip torque on the press tick itself, so the port
  handles the press first.)
- **Landing** clears the flip.

## Considered options

1. **Port it as RocketSim has it** (chosen). `DriveState::flip:
   Option<FlipState>` carries the time and direction.
   `jump::apply_flip_torque` and `jump::advance_flip` follow RocketSim's
   order. The flip torque is added to angular velocity per 120 Hz tick
   (RocketSim divides `flipRelTorque` by its tick-time scale), so no
   inverse-inertia matrix is needed.
2. **Keep the instant kick and add only the damping**: this leaves the
   orientation error and the invented jump-press cancel in place.

## Decision

Option 1. `DODGE_ANGULAR_SPEED` and the jump-press-again cancel are
removed, along with the tests that asserted them. Their replacements test
the real torque, the cancel, damping, pitch lock, the stall, and landing.

## Consequences

- A flip spins at RocketSim's rate and reaches `MAX_CAR_ANGULAR_SPEED` in a
  few ticks. A flipping car's fall stalls as recorded. Pitch-stick cancel
  works per axis: a side flip cannot be pitch-cancelled.
- Behavior change: pressing jump again mid-flip no longer does anything.
  Rocket League has no such mechanic.
- Not done here:
  - air-control magnitudes and damping (`CAR_AIR_CONTROL_TORQUE`/`DAMPING`
    with `CAR_TORQUE_SCALE`); the port's air torque is still a placeholder;
  - the flip's small upward component, if any;
  - auto-flip.

## Validation

- Re-run `rb-verify --self-trace test2.jsonl 4.3 5.0`. vz should stall at
  about -15 uu/s from ~4.47 s, and orientation error through 4.3-4.97 s
  should fall from ~1.6 rad.
