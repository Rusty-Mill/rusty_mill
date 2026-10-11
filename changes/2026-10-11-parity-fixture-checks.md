---
category: Added
changelog: `tools/rb_tape_bot/fixture_checks.py` reproduces the physics constants read off the committed game captures in the parity table's pass 2.
---
## 2026-10-11 - fixture_checks.py: the pass 2 analyses as a script

- **Added:** `tools/rb_tape_bot/fixture_checks.py` (standard library only). Subcommands print the numbers the parity table's pass 2 quotes from the 73 committed captures: speed and angular caps, ball spin saturation, boost tap burn length, ball sliding friction, spin growth and rolling threshold, bounce ratios, and small boost pad trigger distance. `all` runs everything.
- **Changed:** the parity table's pass 2 note now points at the script instead of saying the scripts are not committed; the bounce ratios are stated as both 0.5998 (exact velocities), not 0.5997.
- **Known limitations:** read-only analysis; it does not run the port or compare against it. The pad check covers small pads and cars on the ground (23 pickups), and its hitbox constants are copied from RocketSim as the port uses them.
