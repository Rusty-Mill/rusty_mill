# ADR-0077: A held roll drops the air-control pitch torque

- Status: Rejected (withdrawn the day it was accepted)
- Date: 2026-10-08
- Deciders: baileyrd
- Related: `RB-PHYSICS-001-FR-155`, FR-082, FR-093
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
- Open: roll spin at the 5.5 rad/s cap sprouts yaw/pitch spin in the game.

## Postscript: withdrawn

Further recordings showed the silence is not tied to roll: any tape that held pitch from its
first tick had no pitch for about 25 ticks (pitch alone included), while the same pitch
started 60 ticks in, or 20 ticks after a jump (`probe_air_yaw_pitch`), responded at once. After
tick ~40 the game's car-frame pitch spin grows at the full pitch rate even with roll held. It
is a start-of-capture artifact. FR-155 and its test were removed; `speed_flip`'s bound is back
at 9.4 uu. The fuzz tapes' pitch near the start is the only place it matters.
