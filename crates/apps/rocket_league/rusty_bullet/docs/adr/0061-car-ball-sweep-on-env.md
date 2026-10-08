# ADR-0061: Car-ball hit sweeps as `Env`'s second caller

- Status: Accepted
- Date: 2026-10-06
- Deciders: baileyrd
- Related: RB-VERIFY-003-FR-014, ADR-0060, RB-RESEARCH-O010
- Supersedes/Superseded by: none

## Context

ADR-0060 accepted `rb_env::Env` on the condition that a second caller (a
sweep or a policy) appears, or it folds back into `rb_verify_cli`. The
car-ball hit (`RB-RESEARCH-O010`) is the open residual that most needs
sweeping: the game's hit keeps the car and ball in contact for four ticks.
A scratch sweep (not committed) ran 648 combinations of the hit's numbers;
the constants had to be environment variables for that, and the result was
nearly right for the wrong reason: the window optimum failed the
full-capture gate.

## Decision drivers

- Sweeps must be repeatable and reviewable, not a scratch patch each time.
- The hit's numbers are constants inside `PhysicsWorld`; varying them needs
  a runtime handle, but only for sweeps.
- A prediction restarted cold from one recorded frame (`Env::reset`) loses
  suspension and contact state; measured on `hitjump`, 290 uu/s of car
  velocity error at the default tuning, against effects of tens of uu/s.

## Considered options

1. Keep sweeping with scratch environment-variable patches.
2. Make the hit's numbers a struct on the world (`CarBallTuning`, default
   RocketSim's), settable through `Env`, and add `Env::snap` (keep the
   simulation's memory, move the bodies) and `Env::peek` (roll out on a
   copy). `rb-verify --sweep-hit` is the caller.
3. Rebuild with `reset` per prediction (what the first version of the sweep
   did).

## Decision

Option 2. Option 3 was built first and measured: it reproduces nothing
(`hitjump` default 292 uu/s against 51.5 with tracking), so it is not kept.
`CarBallTuning` is a plain public struct on `PhysicsWorld` with RocketSim's
values as `Default`; no behaviour changes unless a caller sets it.

## Consequences

### Positive

- `Env` has its second caller, so ADR-0060's fold-back trigger is cleared.
- A sweep is one command, about 20 s for 405 tunings over three windows,
  and reproduces the scratch result (default row, same best tuning).
- `snap` and `peek` are the primitives a policy rollout needs as well.

### Negative / tradeoffs

- The window objective can still overfit; the output says it is candidates
  and names the full-capture table as the gate. Nothing automates the gate.
- The pre-roll starts from `Env::reset`, so jump and flip state is the
  default for the first ticks (60 ticks of pre-roll before the first
  prediction); a window right after an air roll or dodge is not trustworthy.
- `CarBallTuning` is public surface on `rb_physics_bullet` that exists for
  tooling.

## Validation and revisit triggers

- Default row reproduces the scratch numbers: done (51.5 against 51.0 uu/s).
- Revisit if the sweep is used for another contact (`snap`/`peek` suggest a
  generic parameter set) or if automating the full-capture gate becomes
  worth it.
