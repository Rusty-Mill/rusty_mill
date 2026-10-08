# ADR-0078: No air throttle while any wheel touches

- Status: Accepted
- Date: 2026-10-08
- Deciders: baileyrd
- Related: `RB-PHYSICS-001-FR-156`, FR-099, FR-148, ADR-0077 (withdrawn)

## Context

`two_wheel_errors.py` groups the one-step velocity error of ticks with one or two wheels
touching by wheel set and throttle sign, in the car's frame. Over the fuzz drives (601..624,
701..724) every wheel pair with the throttle held was short of the game by 0.55 to 0.59 uu/s per
tick forward, and with the throttle reversed long by 0.5 to 0.8: 200/3 uu/s^2 is 0.556 per
tick, the air throttle. The port added it whenever the car was not "on the ground" (fewer than
three wheels down), on top of the touching wheels' own drive.

## Decision

The air throttle is added only with no wheel touching, like air control (FR-099).

## Consequences

- Mean |forward one-step error| on one-wheel ticks 0.500 -> 0.366, on two-wheel ticks 0.611 -> 0.445
  uu/s per tick; three-wheel ticks unchanged. A residual of about 0.35 uu/s per tick on two-wheel
  throttle ticks remains (not explained).
- Random-drive means barely move (113.5 -> 112.1, 102.2 -> 102.5 uu): their errors are dominated
  by landings and bounces.
- Golden gate unchanged.
