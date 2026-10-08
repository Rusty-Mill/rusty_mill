# Parity plan: rusty_bullet against Rocket League

State on 2026-10-07 (spec `RB-PHYSICS-001` 0.150.0, 62 golden recordings).
Evidence: [FIDELITY-SCOREBOARD.md](../research/FIDELITY-SCOREBOARD.md),
[RESEARCH-BACKLOG.md](../research/RESEARCH-BACKLOG.md). This plan orders the
remaining work; it changes no behaviour.

## 1. What "parity" means here

Parity is a number, not a feeling. Four measures, each from a recording of the
real game with the port fed the input the game applied
(`rb-verify --scenario ... --against ... --recorded-inputs`):

| Measure | Today | Parity target |
|---|---|---|
| Targeted mechanics (probes, 62 gated fixtures) | most under 3 uu mean; open ones below | every probe under 5 uu, none on the "does not agree" list |
| Random 6 s drives, mean position error (seeds never tuned on) | 239 uu mean, 104 median (seeds 501-524); 140 / 48 (101-124) | median under 25 uu, and no tape under 10 uu that the game itself repeats |
| Random ball launches | 8.3 uu per 6 s | stay under 10 uu |
| Wall, ceiling and ball-contact tapes (the weak regimes) | wall-start median 132 uu; hit tapes 203 uu mean | median under 40 uu |

The noise floor is smaller than first thought. Workstream F (2026-10-07, 24
tapes x 5 runs, seeds 501-524) found the game repeats itself to 0.0 uu mean on
23 of 24 tapes (`fuzz_505`: 0.0 mean, 97 uu max; `fuzz_521`: 0.1 mean), so on
bot-driven tapes there is no noise to hide behind: the criterion "inside the
game's own spread" reduces to the 5 uu floor. Only rare discrete events differ
between runs (O011 near a wall at the state-set; O020 is therefore an
input-dependent difference, not noise, until a repeat of its tape shows
otherwise). Port error against those recordings: median 103.8 uu best-run mean
(0 of 24 inside 25 uu); the largest are single-event divergences
(`fuzz_512` 1474, `fuzz_508` 1075, `fuzz_501` 527, `fuzz_504` 521 uu).

## 2. Where we are

Agrees (under about 3 uu): throttle, boost, braking, steering, jumps, dodges,
flip cancel, speed flip, half flip, wavedash, air control, tilted landings,
ball bounces (floor, wall, ceiling, corner, goal), car-ball hits, car-car
bumps and demolition, powerslides including reverse ones, a car hitting a side
wall below the lip, dodges with the throttle held.

Does not agree yet, ranked by how often play hits it and how much error it
causes:

| # | Gap | Evidence | Backlog |
|---|---|---|---|
| 1 | Wall and ramp climbing (and falling off a wall) | `wallg_*` 2 to 32 uu by approach angle; wall-start tapes median 132 uu | O012, O019 |
| 2 | Car-ball contact detail: resting ball hit from above, wheels on the ball | `hitfuzz` ball 4 to 260 uu/s on a hit; `car_over_ball` 27 / 44 | O014, O020 |
| 3 | Ceiling | `probe_ceiling` 31 uu | O017 |
| 4 | Boost pads (not in the port at all) | 190 pickups in the recordings | O021 |
| 5 | Demolition respawn (not modelled) | game: 3.0 s later at a spawn point | O018 |
| 6 | Flip into a car | `bumpf_flip` 39 / 65 | O018 |
| 7 | Suspension damper order, rest pose | rest roll 0.003 rad the game does not have | O019 |
| 8 | Smaller residuals, ball hovering when set at rest | `wavedash_early` 7, `half_flip` max 12 | O015, O016 |

Not yet measured at all: three or more cars, a car driving on the ceiling,
kickoff countdown and spawn sequence, goal scoring and replays, the net
(ball into the goal's net), boost pad cooldowns.

## 3. Workstreams

Each workstream ends in measurable exits and follows the repo workflow (branch,
PR, green CI, merge commit; spec FR, traceability, CHANGELOG; ADR where it
chooses between alternatives).

### A. Wall, ramp and ceiling (gaps 1, 3, 7) — highest value

The largest error left and the least understood. Findings so far: per-tick
forces on a steady climb match to 0.1 uu/s; the error enters in the ramp
transition (spikes of about 1 uu/s every 14 to 15 ticks, one per facet edge a
wheel crosses); of the 24 wheel orders only 0123 reproduces 45 degree
climbs, yet it leaves a rest roll the game lacks; reading before the drive
impulses is worse everywhere.

1. (Done 2026-10-07, ADR-0073: `rb-verify --wheel-trace`, `wheel_facets.py`; results in O012.)
   A wheel-ray instrument (and `--wheel-kstep`, whose 30-tick error rises 0.27 uu/s per facet change crossed): record, per tick, which triangle each ray hits
   (port) and compare to the car's per-tick force in the game. A per-tick
   force diff at each facet crossing says whether it is the contact normal, the
   suspension length jump, or the pushback.
2. A clean wheels-on-ramp probe set that does not start with a state-set on
   the wall (the start transient pollutes `wallfuzz`): approach from the floor
   at 10, 20, 30, 45, 60 degrees and 3 speeds (the existing `wallg_*`), plus a
   slow drop onto a wall and a ceiling.
3. Test hypotheses against that set only, not the fuzz: normal smoothing at
   facet seams (the ball already has `adjust_edge_normal`; the wheel rays
   do not), pushback timing, sticky force along a smoothed normal, and the
   O019 ordering with a rest-pose-neutral formulation.
4. Ceiling: a recorded car driving up a wall onto the ceiling and a nose
   landing on it (the current `probe_ceiling` is a floor landing).

Exit: `wallg_*` all under 5 uu, `wall_ride*` and `corner_slide` under 5 uu,
rest roll 0 without breaking any gated fixture, wall-start tape median under
40 uu.

### B. Car-ball contact (gap 2, also 6)

1. Collect the clean single-event data the fuzz found (`hitfuzz`, `bsl_*`)
   and score only one-step predictions (`--self-onestep`), which are exact.
2. Wheels on the ball (O014): the wheel-on-ball interaction needs a recording
   that isolates it (a car dropped wheels-first onto a ball at rest, a few
   speeds); the earlier "reaction on the ball" hypothesis made it worse.
3. Resting ball hit from above (O020): treat as a noise-floor case (see F);
   decide by measurement whether the port should pick the more likely outcome
   or the expected-error one. No physics change until F says what the spread is.
4. Flip into a car (`bumpf_flip`): the dodge's own impulse with a bump in play.

Exit: `car_over_ball` under 5 uu; one-step ball error under 5 uu/s on every
hit event outside the O020 knife-edge cases.

### C. Boost pads (gap 4) — new feature

The port has none, so every long run with boost drifts by 12 per missed
pickup. The data exist in the recordings.

1. Dump `FieldInfo` from the tape bot (exact pad list and kinds) and log the
   pad `is_active` and `timer` per tick from the game packet: this says exactly
   when a pad is taken, which the boost value alone cannot.
2. Drive one pad at many offsets from a fresh match each time (the earlier
   `padpass_*` mixed in the state-set teleport sweeping pads) and fit the
   overlap rule (hitbox against cylinder or box), height, 12 and 100 amounts,
   4 s and 10 s cooldowns.
3. Port into `rb_physics_bullet` as a domain feature with its own tests;
   `rb_env` enables it; ADR (public interface change).

Exit: boost value matches the game to 0.1 across the 190 recorded pickups.

### D. Demolition, respawn, multi-car (gaps 5, 6)

Respawn after 3.0 s at a spawn point the game picks (two seen: (2048, -2560),
(0, -4608); boost 0 or 33). Needs the selection rule, so first: record 10
demolitions and tabulate the spawn point against team and the other cars'
positions. Then three-car and four-car bump scenarios (the bump cooldown is
per pair; never recorded with three cars).

Exit: respawn matches position and boost on 10 of 10 recordings; a 3-car
scenario under 10 uu per car.

### E. Match flow (not yet physics, needed for parity of play)

Kickoff countdown and spawn sequence, goal detection and reset, overtime,
the net's behaviour on a goal. Each needs its own recording; the capture
plugin already sees the game state. Do after A to C, since they matter less
to a bot environment than to a full match.

### F. Measure the game's own spread (done 2026-10-07)

Result: spread 0.0 uu mean on 23 of 24 tapes (see section 1); tool
`spread_report.py`, data `replays/batch_20261007-165122` (local, not
committed). The text below is the plan as written.

Without it the exit numbers above are guesses. Repeat each of 24 fixed
tapes 5 times in the game with identical inputs (the runner supports
`-Repeat`) and report, per tape, the spread between game runs (including the
input start jitter the recorded-input replay removes) and the port's error
against the nearest run. Parity on a tape means the port is inside the
game's spread. This also decides O020 and sets the real floor for the
random-drive target.

### G. Process and tooling

- Keep the three random-tape generators as the regression benchmark: the
  holdout seeds (501-524 and any new range) are only for reporting, never for
  tuning; report mean and median.
- Add a CI-run smoke of the random-tape benchmark? Not recommended: needs
  the game. Keep it a local gate (`docs/research/FIDELITY-SCOREBOARD.md` updated
  on each physics PR).
- The owner's original keyboard recordings (`test2`, `hitjump`, `front`,
  `side`) are not on the development machine; FR-139 to FR-148 were not checked
  against them. Re-run that gate once, on the machine that has them, before
  parity is claimed for keyboard play.
- Capture plugin: record `DodgeForward` / `DodgeStrafe` (removes the
  keyboard-capture flag of ADR-0070) and the pad state.

## 4. Order and rough sizing

Part-time cycles (one cycle is roughly a session of a few PRs):

| Step | Work | Cycles |
|---|---|---|
| 1 | F: game spread measurement (done: spread 0.0 uu on 23 of 24 tapes) | 0.5 |
| 2 | C step 1 and 2: pad data (fits on the same tape-bot runs as F) | 1 |
| 3 | A steps 1 and 2: instrument and clean probes | 1 to 2 |
| 4 | C step 3: pads in the port (done, ADR-0072, FR-149) | 1 |
| 5 | A step 3 and 4: wall and ceiling fixes | 2 to 4 (largest uncertainty) |
| 6 | B: ball contact | 1 to 2 |
| 7 | D: respawn and multi-car | 1 to 2 |
| 8 | Owner gate re-run, scoreboard, status refresh | 0.5 |
| 9 | E: match flow | 2+, only if a full match is the goal |

Physics parity (steps 1 to 8) is about 8 to 14 cycles, dominated by A, which may
turn out to be a handful of facet-crossing details or one structural
mistake; the instrument in A.1 is what decides.

## 5. Decisions (owner, 2026-10-07)

1. Parity is measured for bot-driven cars only; keyboard captures stay a legacy
   mode (ADR-0070 flag) plus the one-time gate check of G.
2. Physics only for now; E (match flow) stays last and optional.
3. Boost pads on by default in `Env`, off in `PhysicsWorld::new` (existing
   fixtures and the golden gate unchanged).
4. Unattended overnight game runs are acceptable (C, D and F, about 10 minutes
   per 24 tapes).

## 6. Risks

- The noise floor may sit above some targets (O020); F bounds this early.
- Wall behaviour may be coupled to the damper order (O019); changing one may
  move the other, so A keeps both under the same probe set.
- Unrecorded regimes (ceiling driving, 3+ cars, kickoffs) may hide further
  gaps: each workstream starts with a fuzz of its regime before fixing.
- The golden gate guards only what is recorded; new fixtures are added with
  every fix so the next change cannot silently undo it.
