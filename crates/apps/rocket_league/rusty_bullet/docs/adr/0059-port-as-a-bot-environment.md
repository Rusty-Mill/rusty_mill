# ADR-0059: The physics port as a stepping environment for bots

- Status: Proposed
- Date: 2026-10-06
- Deciders: baileyrd
- Related: RB-RESEARCH-O005, RB-RESEARCH-O007, ADR-0056, ADR-0057,
  `docs/research/BOT-CAPTURE-PLAN.md`
- Supersedes/Superseded by: none

## Context

Bots need somewhere to run. The real game is slow, needs a human, and is
offline-only. RocketSim, this project's reference engine, is what the
community trains RLGym-style policies against because it is fast and
deterministic. `rb_physics_bullet` is a port of the same mechanics, scored
against recordings, and `simulate_scenario` already steps it from a scenario
with scripted inputs. What is missing is a *stepping* surface a policy can
call: reset to a state, apply an action, step, observe. Today the only
callers are recording-driven loops (`simulate`, `simulate_recorded*`) and
scripted scenarios.

Measured today (2026-10-06): runs are deterministic (identical output across
runs, no hash-order iteration in the physics crate); `--self` over 2818
frames takes 0.35 s wall including startup. Steps per second has not been
benchmarked.

## Decision drivers

- Train and sweep without the game; keep the port as the thing being scored.
- No new dependency and no abstraction before a second real call site.
- `rb_domain` stays free of I/O and of any bot framework.
- Fidelity limits must stay visible: the port omits boost pads, goals and
  scoring, kickoff, demolitions and ball prediction.

## Considered options

1. **Do nothing**: scenarios only. Fits mechanic spikes, cannot train.
2. **A small `Env` in a new adapter crate `rb_env`** over `PhysicsWorld`:
   `reset(&Scenario)`, `step(&[ControllerInput]) -> Observation`, with
   `Observation` built from `PhysicsFrame`. Pure computation, depends on
   `rb_domain`, `rb_physics_bullet`, `rb_scenario`.
3. **Speak the RLBot v5 protocol** (O007 reading B) so any v5 bot runs
   unchanged. Larger (schema, sockets, packet pacing) and it is needed
   anyway only if existing bots must run as they are.
4. **A Python gym binding** (PyO3). Adds a toolchain and a dependency
   boundary; defer until a policy actually exists.

## Decision

Proposed: option 2 first, as the smallest thing that gives both the scenario
loop and a future policy loop one call surface. Option 3 sits on top of it
later (a protocol adapter calling `Env`), so nothing in option 2 is thrown
away. Options 1 and 4 are not chosen.

Sketch (not final): `Env::new(arena)`; `reset(&Scenario) -> Observation`;
`step(inputs) -> (Observation, Done)` advancing one 120 Hz tick; `Done`
covers tape end and caller-set horizon only (no goals yet). Reuse
`simulate_scenario`'s loop as the first caller so the existing scenarios
become the first tests: `Env` must reproduce `simulate_scenario` frame for
frame.

## Consequences

### Positive

- One stepping surface for scripted tapes, sweeps and policies.
- Existing scenarios double as the acceptance tests.
- Keeps O007 reachable without rework.

### Negative / tradeoffs

- A new crate before a second caller: justified only if a sweep or policy
  caller is written alongside it; otherwise the call surface should stay in
  `rb_verify_cli`.
- Training in the port optimises against the port's errors; a policy that
  works there may fail in the game, so game captures remain the check.
- Missing rules (pads, goals, kickoff) bound which tasks can be trained.

## Validation and revisit triggers

- Benchmark `step` first (target to be set after the first measurement;
  RocketSim-class tools do thousands of steps per second per core).
- `Env` reproduces every shipped scenario exactly versus
  `simulate_scenario`; determinism test runs a scenario twice and compares.
- Revisit if the first game captures show the port's error on a trained
  mechanic is larger than the policy's margin, or if no second caller
  appears within the next two delivery cycles (then fold back into
  `rb_verify_cli`).
