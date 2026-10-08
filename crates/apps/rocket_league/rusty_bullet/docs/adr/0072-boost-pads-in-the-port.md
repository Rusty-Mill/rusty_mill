# ADR-0072: Boost pads in the port, on in `Env`, off in `PhysicsWorld::new`

- Status: Accepted
- Date: 2026-10-07
- Deciders: baileyrd
- Related: `RB-PHYSICS-001-FR-149`, `RB-RESEARCH-O021`, ADR-0071, `docs/roadmap/PARITY-PLAN.md` (workstream C)
- Supersedes/Superseded by: none

## Context

The game has 34 boost pads (six big, 28 small); the port had none, so every
recording with a pickup drifted by 12 or 100 boost. Workstream C measured the
pickup rule on the game itself (ADR-0071, `pad_fit.py`, 153 runs, 63 small and
24 big pickups, about 11800 passes without one).

## Decision drivers

- The 62 golden fixtures and every existing `PhysicsWorld` caller must not move.
- A policy plays on a field with pads, so the stepping environment should have them.
- The rule must come from the game's data, not from RocketSim's constants.

## Considered options

1. Pads always on in `PhysicsWorld` (moves fixtures that pass over a pad; no way to compare).
2. Pads as a field of `PhysicsFrame` (public domain type change, every adapter).
3. Pads in `rb_physics_bullet` behind `set_boost_pads`, on by default in `Env` only.

## Decision

Option 3. A `pads` module holds the 34 pads (game order, by y then x) and the
rule; `PhysicsWorld::set_boost_pads(bool)` and `boost_pads()` expose them;
`Env::set_boost_pads` (default true) and `Env::boost_pads()` pass it on, and
`reset` makes every pad active. The rule (fitted):

- the car's origin within 176 uu (small) or 208 uu (big) of the pad in the
  plane and at most 160 uu above it;
- tested a quarter of the way from the tick's start position to its end
  position (the logged pickups fit a position 0.75 of a tick before the packet's);
- a small pad adds 12 (cap 100), a big pad fills the tank, and a full tank
  leaves the pad alone;
- back after 4 s (small) or 10 s (big), counted in ticks (480 and 1200).

`PhysicsFrame` is unchanged: pad state is read through `boost_pads()`, and a
pickup shows in the car's `boost_amount`.

Addendum (2026-10-08): a joint fit of the test point and both radii in the port itself, on the
34 probe recordings (a grid over fraction 0.5 to 0.7, small radius 174 to 178, big 206 to 210,
scoring the tick of every pickup and every pickup / no-pickup decision), keeps 176 and 208 and
moves the test point from 0.25 to 0.6 of the tick: 20 of 21 pickups on the game's tick, 1 late
(was 16 exact, 5 late), no decision mismatch. The log fit of O021 had the time reference wrong
by a fraction of a tick because it matched events to rows of the capture.

## Consequences

### Positive

- Boost no longer drifts in a long run; on the 34 probe tapes 32 pickup
  decisions match the game (the other two are the 95-boost probes, decided by the
  state-set sweep before the tape began) and every gain is the same.
- `rb-verify --scenario` scores `boost error`, and `RB_BOOST=1` lists the ticks
  at which each side's tank changes.

### Negative / tradeoffs

- 5 of 21 probe pickups come one tick late (16 are on the same tick); the fit has a 0.7 uu gap at the
  chosen fraction (the whole previous row is 2.4 uu).
- Unmeasured and assumed: the cap when a small pad would pass 100, the first car
  in index order wins a pad two reach in a tick, no lower height limit, pads
  reached in the air at a wall or ceiling.
- A recording's first ticks can hold pads the game's state set swept; the replay
  seeds the tank from the recording's first aligned frame.
- Callers that built an `Env` and compared against `PhysicsWorld::from_frame`
  now see pads on one side (`rb_env` tests use a full tank where this matters).

## Validation and revisit triggers

- Unit tests in `pads`, `world` and `rb_env`; 488 tests in `rb_physics_bullet`.
- `rb-verify --scenario ... --against ... --recorded-inputs` on `padp_*` and fuzz batches.
- Revisit when pads in the air, the cap, or two cars on one pad are measured, or
  if the 1-tick lag matters to a policy.
