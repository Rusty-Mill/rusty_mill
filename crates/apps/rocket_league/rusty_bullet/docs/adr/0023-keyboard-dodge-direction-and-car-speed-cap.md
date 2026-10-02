# ADR-0023: A dodge's forward part follows throttle when pitch is centred; car speed capped

- Status: Accepted
- Date: 2026-10-02
- Deciders: baileyrd
- Related: RB-PHYSICS-001-FR-103, FR-059, FR-082, FR-087; ADR-0013,
  ADR-0014
- Supersedes/Superseded by: none

## Context

`rb-verify --self-onestep` ranked two `test2.jsonl` jump presses next
after ADR-0022: 6.058 s (432 uu/s) and 12.55 s (396 uu/s). Both are
airborne dodges recorded with yaw ±1, pitch 0 and throttle 1. The
candidate dodged purely sideways, as RocketSim does from
`dodgeDir = (-pitch, yaw + roll)`. The recorded cars gained 354 uu/s
forward as well, the exact impulse of a diagonal dodge (`0.707 * 500`
forward, the side part speed-scaled).

All 15 airborne dodges in the owner's three captures fit one rule: pitch
past the deadzone sets the forward part (as before); otherwise throttle
does. Yaw-only presses without throttle (`side.jsonl`, `test2` 8.267 s)
dodge purely sideways. Rocket League dodges from its own `DodgeForward`
and `DodgeStrafe` inputs, which the captures don't record. On the owner's
keyboard bindings, `DodgeForward` evidently follows throttle.

At 12.55 s the recorded car leaves the dodge at exactly 2300 uu/s, along
the direction the uncapped dodge would give. RocketSim caps car speed at
`CAR_MAX_SPEED` and spin at `CAR_MAX_ANG_SPEED` every tick. The port only
capped spin.

## Decision

- `drive::jump::dodge_direction`: the stick still decides whether a press
  dodges. Its forward part is `-pitch` when pitch is past the deadzone,
  else the throttle.
- `drive::clamp_velocity` (was `clamp_angular_speed`) also caps linear
  speed at `MAX_CAR_SPEED`, at the end of the step, as FR-087 placed the
  spin cap.

## Consequences

- Both presses match: one-step error 0.0 and 0.5 uu/s, spin 0.00.
  `test2.jsonl` mean one-step error 2.72 uu/s (was 3.10).
- A controller player holding the throttle trigger with the stick
  sideways would get a diagonal dodge here, which real Rocket League may
  not give. Recording `DodgeForward`/`DodgeStrafe` in the capture plugin
  would remove the inference. That changes the capture format, so it is
  left as a proposal.
- Whether throttle alone (neutral stick) starts a dodge is unknown: no
  capture has such a press. It is kept as a double jump.
