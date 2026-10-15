---
category: Fixed
changelog: rb_physics_bullet follows the game on slow-car braking, boost drain order, the throttle deadzone and analog input quantization; the tape bot batch starts BakkesMod when it is not running.
---
## 2026-10-15 - rb_physics_bullet: brake, boost order, deadzone, input quantization

- **Fixed:** the wheel brake is `clamp(K * v, +-B) * 60 * dt` per wheel with no effective-mass term, so a slow
  coasting car overshoots zero and rings (per-tick speed ratio -0.2638 in the model; of 363 captured coasting ticks
  between 2 and 10 uu/s, 80 % form a tight cluster at -0.2674, median -0.2679, i.e. a constant of 114.07 against the
  113.74 used: 0.3 %) instead of stopping dead. Fast braking is unchanged (3500 uu/s^2). Batch `20261008-111216`: fuzz families mean
  of means 151.0 to 140.3 uu, duels 574.6 to 516.5 uu; the holdout is neutral (132.3 to 132.4 uu).
- **Fixed:** boost drains from the second tick of a burn, before that tick's push. The activation tick is free and
  the tick that empties the tank does not push; pushing ticks are unchanged (361 for a full tank at 120 Hz).
  Captures agree (the first push precedes the first drain). Tank error is closer on 90 of 96 tapes, worse on none.
- **Fixed:** the pedal throttle deadzone is `|throttle| <= 0.01` (was 0.001), so the smallest nonzero input
  (1/127) coasts. Captured: from rest a throttle of 0.008 (one byte step) leaves the car still and 0.016 (two steps)
  drives it at 24.7 uu/s^2, which brackets the threshold in [0.0079, 0.0157); 0.01 is inside.
- **Fixed:** analog inputs are quantized as the game does (`drive::quantize_input`, applied in
  `PhysicsWorld::set_car_input`): clamp to [-1, 1], round-trip through one byte (negative steps 1/128, positive
  1/127), so +0.5 reads 64/127 and -0.5 stays -0.5. Confirmed on captures (`quant_*` and `quantlvl_*` tapes,
  `capture_checks.py`): the spin of a held +v over a held -v on roll is 1.0080, 1.0080, 1.0079, 0.9967 and 0.9991 at
  v = 0.1, 0.3, 0.5, 0.7 and 0.9, the rule's prediction at every level (the signs round differently, so a symmetric
  or absent quantizer gives 1.0000), and 1.008 to 1.009 on pitch and yaw at 0.5; the port now reproduces the
  0.5 captures to 0.00 rad/s.
- **Added:** `run_batch.ps1` starts BakkesMod when it is not running (`-BakkesMod <path>`, `-NoBakkesMod`); the
  six `quant_*` and eight `quantlvl_*` probe tapes, four `dz_*` deadzone tapes, `gen_quant_probe.py`,
  `quant_ratio.py` and `capture_checks.py`.
- **Known limitations:** 71 of the 363 coasting ticks fall outside the cluster (57 lose less speed, 14 more) and are
  not explained; the fuzz tapes are chaotic around the input quantization change (batch mean of means
  140.3 to 159.8 uu with a median per-tape change of 0; holdout 132.4 to 126.4 uu), and the four holdout tapes that
  moved most after the brake change sit on a hard 25 uu/s pedal threshold (threshold luck, not a rule error). The
  recorded input echoed in `frame()` is the quantized value. The `rusty_bullet` subfolder's own `CHANGELOG.md` is
  not edited (this fragment is the live log).
