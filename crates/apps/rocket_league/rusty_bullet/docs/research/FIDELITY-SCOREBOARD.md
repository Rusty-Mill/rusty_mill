# Fidelity scoreboard: the port against real Rocket League

State on 2026-10-08 (spec `RB-PHYSICS-001` 0.155.0; the rows added since 2026-10-07 are marked). Every number is the mean
position error in uu between the port, fed the input the game recorded
(`rb-verify --scenario S --against C --recorded-inputs`), and a tape-bot
recording of real Rocket League: the car, and where it matters the ball and a
second car. The recordings are `replays/` (gitignored); 62 of them, trimmed to
their tapes, are in `tools/rb_tape_bot/fixtures/` and fail `cargo test` if the
port gets worse (ADR-0066). How it was measured: `tools/rb_tape_bot/README.md`;
the evidence for each row: `BOT-RUN-SHEET.md` (session 3) and
`RESEARCH-BACKLOG.md`.

## Agrees (under about 3 uu mean)

| Mechanic | Scenarios | Mean error (uu) |
|---|---|---|
| Throttle, boost, coasting, braking, reversing | `probe_accel`, `accel_boost`, `coast`, `brake`, `reverse` | 0.2 to 0.6 |
| Steering | `turn_slow` 2.7, `turn_fast` 3.0, `turn_half_boost` (to the wall) | 1.4 to 3 |
| Jumps, double jumps, the second-jump window | `jump_short/mid/long`, `double_jump`, `late_dodge` | 0.4 to 1.4 |
| Dodges in every direction, flip cancel, air roll after a flip, a dodge with the throttle held at 1500 uu/s | `dodge_back/side/diag`, `dodge_fast_*`, `flip_cancel`, `flip_air_roll`, `dodgeth_*` | 0.2 to 1.6 |
| Speed flip, half flip, wavedash (mid, late), pogo, hard landing | shipped scenarios | 0.3 to 2.6 |
| Air control, boost in the air | `air_roll`, `air_yaw_pitch`, `boost_air_*` | 1.0 to 2.5 |
| Landings on the wheels, tilted | `land_wheels`, `land_tilted` | 0.2 to 0.3 |
| Ball: floor, wall, ceiling, corner, goal, rolling, spin | `probe_ball_*` | 1.4 to 4.2 (ball) |
| Car-ball hits: throttle, boost, off-centre, glancing, side | `hit_*`, `nose_hit_glancing`, `ball_bounce_car_side` | 0.1 to 0.6 car, 0.3 to 1.4 ball |
| A car landing on the side wall below the lip (box edge on a mesh edge) | `wall_land` 4.4 | under 5 |
| Powerslide: straight, half-left, release, long reverse slides; reverse + handbrake | `ps_straight` 0.3, `ps_half_left` 1.1 to 4.3, `ps_release` 2.8 to 5.1, `powerslide` 4.8, `ps_boost` 4.9, `hb_rev_*` 0.2 | under 8 |
| **Car-car bumps**: rear (300 to 2100 uu/s), side, head-on, off-centre, moving, crossing, in the air | 36 `bump*` scenarios | 0.1 to 11 per car |
| **Demolition** (supersonic nose on an enemy; teammates only bumped) | `bumpd_*` | enemy 0.0 until removed, attacker about 1 |
| **Respawn after a demolition** (new 2026-10-08): out exactly 3.000 s, back at a kickoff spawn point (random; the recording's pick is replayed) | `demo_0` to `demo_11` | victim 0.1 to 0.3 over 700 ticks |
| **Boost pads** (new 2026-10-08): pickups, amounts, respawn | `padp_*` (34 probes), fuzz tapes | 32 of 34 pickup decisions match, 20 of 21 on the game's tick |
| **Airborne bumper on a grounded car** (new 2026-10-08) | `bumpag_*` | victim 14 to 18 uu at 700 to 1400 uu/s (was 50 to 70) |
| **Three cars**: demolish two in a row, a bumped teammate into an enemy | `tri_double_demo`, `tri_mate_into_enemy` | 1.2 to 1.8 attacker, 0.0 to 0.1 and 9 to 29 for the others |
| **Plain car-ball hit on a resting ball** (new 2026-10-08) | `hitfuzz_307/308/312/317/320` | post-hit speed 0.1 to 1 percent fast |

## Does not agree yet

| Mechanic | Mean error (uu) | Why, and where |
|---|---|---|
| Climbing a wall or ramp | `wall_ride` 9, `wall_ride_slow` 16, `wall_ride_boost` 12 to 19, `corner_slide` 22 | The port climbs about 10% slowly; a family of suspension and pushback constants trades errors (O012) |
| A car hitting the ceiling | `ceiling` 31 | A nose landing on the ceiling plane; not yet traced (O017) |
| A resting ball hit from above by a car | `hitfuzz_304/306/315` ball 42 to 391 uu | Deterministic but split by one tick of contact time in the game (five repeats each): the floor's push back on the hit tick (O020) |
| Wheels on the ball | `car_over_ball` 34 / 62; `wob_*` drops car 22 / ball 69 mean | Pushback reads the ball's velocity (FR-150, car 28 -> 22); a one-sided reaction on the ball trades car for ball (O014) |
| A flip into a car | `bumpf_flip` 20.6 / 43.8 (was 202 / 341 at the honest alignment) | No bump when the nose points down (FR-152); the contact is a tick early |
| Two cars into one, or a pinch | `tri_two_on_one` 244 / 292, `tri_pinch` 17 / 122 | Secondary collisions between cars already moving (O018) |
| The game's own repeatability (new 2026-10-08) | 23 of 24 random tapes repeat to 0.0 uu | The port's 104 uu median is all port error |
| A ball set at rest in the air | the game's ball hovers | State-setting artefact (O016) |

Smaller: `wavedash_early` 7 (a chaotic hop), `half_flip` 2.4 / 11.8 max,
`turn_fast` 11 max.

## Random drives (the holistic number)

Seeded random 6 s controller tapes (`gen_fuzz.py`; driving, boost, handbrake,
jumps, dodges, air control), scored with the recorded input, mean position error
per tape: seeds 1 to 24 (used to find gaps) 190 -> 160 uu mean of means (median
95 uu); seeds 101 to 124 (held out, never tuned on) **140 uu mean of means,
median 48 uu**, none under 10 uu throughout. They are chaotic: a 0.5 uu/s
difference across a brake/engine switch at |forward speed| 25 costs 13 uu/s in
one tick, and a wall or goal-post hit amplifies. Before and after, on 24 tapes never looked at (seeds 501 to 524): the port at
commit `97ec2e0` (before FR-144) 354 uu mean of means (median 143), now **239 uu
(median 104)**, nine tapes better and six worse by over 1 uu. The smoother number is the
15-tick-ahead velocity error from each recorded frame over the 48 tapes: mean
**5.2 uu/s** (median 4.5). Every calibrated constant tried (handbrake lateral
grip, powerslide steer, rise and fall rates, lateral curve end, pushback ERP,
sticky force) worsens it when moved: the remaining error is structural, not a
constant.

Latest (2026-10-08, after FR-156 and FR-157): fresh seeds 601 to 624 **114 uu mean of means,
median 34 uu**; seeds 701 to 724 (never looked at before the check) **105 uu, median 25 uu**, 16
of 24 under 30 uu; the six worst tapes are landings and bounces. Systematic one-step bias by
regime (`regime_bias.py`, 96 tapes) is now under 0.02 rad/s per tick in spin everywhere and under
0.4 uu/s per tick in velocity (the 4-wheel resting 0.2 uu/s per tick is the game's resting
velocity report); touchdowns (`touchdown_errors.py`, 195 landings) have a median error of 0.2 to
0.7 uu/s over four ticks. The remaining error is events: bounces on geometry, landings with spin,
hits on a resting ball.

Latest (2026-10-08, after FR-154 to FR-159; fresh seeds `wallfuzz_421` to `444`): **mean 121 uu, median
43 uu**, 9 of 24 under 30 uu: the wall-start regime is no longer the weakest. Fresh hit tapes
(`hitfuzz_401` to `424`): 14 of 24 have a ball error of 0.0, five are over 70 uu (the resting-ball knife
edge, O020). Two-car duels (`duel_*`, held-out seeds 941 to 964): median 157 uu, total 11900 uu against 15514
before the bump rules FR-158 and FR-159.

The game is not repeatable once two cars touch (`spread_report.py` on `duel_941` to `952`, 3 runs each, 2026-10-08):
the median spread between the game's own runs is 63 uu (up to 439 mean), the port's best-run error median 30 uu, and
**8 of 12 tapes are inside the game's own spread**. The four outside: `duel_946`, `duel_950` (game repeats to 0.0, a plain
drive that goes wrong in the port at tick 150 to 290, 87 uu), `duel_944` (a head-on whose bump matches to 3 to 16 uu/s and
whose random tape then amplifies it), `duel_952`. First-tick bump errors in 98 first-contact events are mostly under
3 uu/s along and across the bumper's heading; at 1500 to 2100 uu/s the port's upward kick is 13 to 36 uu/s high
(the game's saturates near 345 uu/s).

## Random ball launches

`gen_ball_fuzz.py`: 24 seeded 6 s launches (random place, velocity 500 to 2500 uu/s,
spin up to 5 rad/s; the car parked and idle), bouncing off the floor, walls,
ceiling, corners, goals and posts: mean ball position error per tape **8.3 uu**
(median 8.0, worst tape 15.4, worst moment 53 uu). The one-step ball prediction
from every recorded frame is exact to 0.1 uu, including the bounces; the drift is
0.01 uu per tick of accumulated rounding. Two tapes are in the golden gate.

## Random drives that start at the ball

`gen_hit_fuzz.py`: 24 seeded runs at the ball, then 6 s of random driving
(`hitfuzz_301` to `324`): car mean of means 203 uu (median 89), ball 77 uu; half
the tapes never touch the ball. The one-step prediction of every recorded
car-ball hit shows the gap: a resting ball hit from above (O020); the rest of the
hits agree.

## Random drives that start on a wall

`gen_wall_fuzz.py` (`wallfuzz_401` to `416`): the car state-set on a side wall at
a random height and speed, then 6 s of random driving up the wall, onto the
ceiling and off it with jumps: mean position error per tape **median 132 uu,
mean 284 uu** (4.8 to 1613); the weakest regime by far (wall climbs O012, the
ceiling, falling off a wall). The first ticks after the state-set already differ
(the car on a wall falls 15 uu/s faster in the game over the first four ticks),
so these tapes mix a start transient with the climb; use the isolated
`wallg_*` and `wall_ride*` probes to measure a fix.

## What changed on 2026-10-07

- The recorded-input replay (`--recorded-inputs`) removed the tape bot's
  variable start delay, so most of the earlier "gaps" were timing, not physics:
  `speed_flip` 87 -> 0.8, `brake` 18 -> 0.3, the dodges 9 to 11 -> 0.5.
- Real fixes: a second jump press within 6 ticks is ignored (FR-139,
  `speed_flip`); car bumps (FR-140, FR-143); box-box contacts as `dBoxBox`
  (FR-141: clipped faces and the 1.05 edge fudge factor); demolition (FR-142).
- Later the same day: the engine fades with the absolute forward speed (FR-144),
  the boost minimum burn (FR-145, FR-148), box against a triangle's edge or
  vertex (FR-146), a dodge ignores the throttle unless it is a keyboard capture
  (FR-147). Open: O019 (suspension damper order: the rest pose is wrong by 0.003
  rad of roll, fixing it breaks wall rides), wall climbs (O012), wheels on the
  ball (O014).
- Tooling: unattended runs (ADR-0064), two-car scenarios (ADR-0067), the
  golden-capture gate (ADR-0066), plugin 1.5 (RLBot boost recorded), fuzz tapes
  and the contact-impulse reader (`tools/rb_tape_bot`).

## Not covered by any recording

The owner's `test2`/`hitjump`/`front`/`side` recordings (the original k = 30
gate) were not on the machine that did this work; changes were checked against
the 47 tape-bot fixtures instead. Run the old gate before relying on FR-139 to
FR-143 for anything those recordings cover. Never recorded: boost pad pickup,
kickoff countdown, scoring and goal replays, a car on the ceiling, three or
more cars, a demolished car's respawn.
