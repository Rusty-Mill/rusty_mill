# Fidelity scoreboard: the port against real Rocket League

State on 2026-10-07 (spec `RB-PHYSICS-001` 0.146.0). Every number is the mean
position error in uu between the port, fed the input the game recorded
(`rb-verify --scenario S --against C --recorded-inputs`), and a tape-bot
recording of real Rocket League: the car, and where it matters the ball and a
second car. The recordings are `replays/` (gitignored); 54 of them, trimmed to
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
| Dodges in every direction, flip cancel, air roll after a flip | `dodge_back/side/diag`, `dodge_fast_*`, `flip_cancel`, `flip_air_roll` | 0.2 to 1.3 |
| Speed flip, half flip, wavedash (mid, late), pogo, hard landing | shipped scenarios | 0.3 to 2.6 |
| Air control, boost in the air | `air_roll`, `air_yaw_pitch`, `boost_air_*` | 1.0 to 2.5 |
| Landings on the wheels, tilted | `land_wheels`, `land_tilted` | 0.2 to 0.3 |
| Ball: floor, wall, ceiling, corner, goal, rolling, spin | `probe_ball_*` | 1.4 to 4.2 (ball) |
| Car-ball hits: throttle, boost, off-centre, glancing, side | `hit_*`, `nose_hit_glancing`, `ball_bounce_car_side` | 0.1 to 0.6 car, 0.3 to 1.4 ball |
| Powerslide: straight, half-left, release, long reverse slides; reverse + handbrake | `ps_straight` 0.3, `ps_half_left` 1.1 to 4.3, `ps_release` 2.8 to 5.1, `powerslide` 4.8, `ps_boost` 4.9, `hb_rev_*` 0.2 | under 8 |
| **Car-car bumps**: rear (300 to 2100 uu/s), side, head-on, off-centre, moving, crossing, in the air | 36 `bump*` scenarios | 0.1 to 11 per car |
| **Demolition** (supersonic nose on an enemy; teammates only bumped) | `bumpd_*` | enemy 0.0 until removed, attacker about 1 |

## Does not agree yet

| Mechanic | Mean error (uu) | Why, and where |
|---|---|---|
| Climbing a wall or ramp | `wall_ride` 9, `wall_ride_slow` 16, `wall_ride_boost` 12 to 19, `corner_slide` 22 | The port climbs about 10% slowly; a family of suspension and pushback constants trades errors (O012) |
| A car landing on a wall or hitting the ceiling | `wall_land` 77, `ceiling` 31 | Car-body contact against the ramp mesh from the air differs; a uniform material cannot fix it (O017) |
| Wheels on the ball | `car_over_ball` 27 car / 44 ball | Ordinary hits agree; the wheel-on-ball contact does not (O014) |
| A flip into a car | `bumpf_flip` 39 / 65 | The dodge's own impulse is in play |
| Respawn after a demolition | not modelled | The game brings the car back after exactly 3 s at a spawn point it picks (O018) |
| A ball set at rest in the air | the game's ball hovers | State-setting artefact (O016) |

Smaller: `wavedash_early` 7 (a chaotic hop), `half_flip` 2.4 / 11.8 max,
`turn_fast` 11 max.

## What changed on 2026-10-07

- The recorded-input replay (`--recorded-inputs`) removed the tape bot's
  variable start delay, so most of the earlier "gaps" were timing, not physics:
  `speed_flip` 87 -> 0.8, `brake` 18 -> 0.3, the dodges 9 to 11 -> 0.5.
- Real fixes: a second jump press within 6 ticks is ignored (FR-139,
  `speed_flip`); car bumps (FR-140, FR-143); box-box contacts as `dBoxBox`
  (FR-141: clipped faces and the 1.05 edge fudge factor); demolition (FR-142).
- Tooling: unattended runs (ADR-0064), two-car scenarios (ADR-0067), the
  golden-capture gate (ADR-0066), plugin 1.5 (RLBot boost recorded).

## Not covered by any recording

The owner's `test2`/`hitjump`/`front`/`side` recordings (the original k = 30
gate) were not on the machine that did this work; changes were checked against
the 47 tape-bot fixtures instead. Run the old gate before relying on FR-139 to
FR-143 for anything those recordings cover. Never recorded: boost pad pickup,
kickoff countdown, scoring and goal replays, a car on the ceiling, three or
more cars, a demolished car's respawn.
