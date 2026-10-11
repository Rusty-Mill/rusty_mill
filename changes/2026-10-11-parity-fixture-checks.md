---
category: Added
changelog: `tools/rb_tape_bot/fixture_checks.py` reproduces the physics constants read off the committed game captures in the parity table's pass 2.
---
## 2026-10-11 - fixture_checks.py: the pass 2 analyses as a script

- **Added:** `tools/rb_tape_bot/fixture_checks.py` (standard library only) and `test_fixture_checks.py`. Subcommands print the numbers the parity table's pass 2 quotes from the 73 committed captures: speed and angular caps (dynamic plateaus, not seeded starts), boost tap burn length, ball sliding friction with exponential drag removed, spin growth, rolling threshold, bounce ratios, and the origin-to-pad distance at small pad pickups. A malformed record in the middle of a capture is an error with file and line; only a truncated last line is skipped.
- **Changed:** the parity table's pass 2 now points at the script and carries the review corrections: the pad finding is restated as an initial-acquisition discrepancy (RocketSim's 144 uu origin cylinder against first pickups at 145 to 172 uu; the 120 uu box is a retention test and the earlier counterfactual is withdrawn, F3 stays open), the cap evidence uses dynamic plateaus, spin saturation is separated from the rolling transition, the rebounds are labelled ceiling and floor, friction is about 228 uu/s², and `ja_thr`'s 11 drain steps are left unexplained.
- **Known limitations:** read-only analysis; it does not run the port or compare against it. The pad check covers small pads and cars on the ground (23 first pickups) and measures acquisition only; the tests are not run in CI (run them by hand, see the script docstring).
