# ADR-0077: A held roll drops the air-control pitch torque

- Status: Accepted
- Date: 2026-10-08
- Deciders: baileyrd
- Related: `RB-PHYSICS-001-FR-155`, FR-082, FR-093, `RB-RESEARCH-O023`
- Supersedes/Superseded by: none

## Context

The random drives' first divergences were mostly in the air, several with roll held together
with other sticks. `gen_air_combo.py` recorded a level car at rest at z = 1000 under held
combinations. Yaw + roll matched. Every combination with pitch + roll spun about the roll axis
only in the game, where the port also pitched.

## Decision

`apply_air_control` zeroes the pitch torque when the roll stick is non-zero. The pitch damping
fade keeps using the pitch stick.

## Consequences

- Positive: `aircombo_pitch_roll` and the other pitch + roll tapes 0.1 uu (spin error 2.4 rad/s
  before); boosted pitch + yaw + roll 44 -> 15 uu mean.
- Negative: the keyboard `speed_flip` capture held pitch and roll together and the game kept the
  pitch; its golden bound grew from 9.4 to 72 uu max. Keyboard captures are legacy (owner
  decision 1, 2026-10-07).
- Open: roll spin at the 5.5 rad/s cap sprouts yaw/pitch spin in the game (O023).
