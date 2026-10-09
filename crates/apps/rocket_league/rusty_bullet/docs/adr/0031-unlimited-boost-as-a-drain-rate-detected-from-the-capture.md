# ADR-0031: Unlimited boost is a drain rate of 0, detected from the capture

- Status: Accepted
- Date: 2026-10-02
- Deciders: baileyrd
- Related: RB-PHYSICS-001-FR-111; RB-VERIFY-003-FR-009; ADR-0030
- Supersedes/Superseded by: —

## Context

The owner's captures are freeplay with unlimited boost: every capture's
fuel stays at 100. RB-VERIFY-003-FR-009 made snapped predictions take the
recorded fuel. A free run (`--self`), though, still drained the
candidate's own tank, so `test2`'s free run lost boost partway through.

Two decisions:
- How to represent unlimited boost: a bool, or the drain rate.
- How the verifier learns about it: a CLI flag, or detection from the
  capture.

## Decision

- **Representation: the drain rate.** It mirrors RocketSim's
  `MutatorConfig::boostUsedPerSecond`, so one number covers both default
  and unlimited boost, plus any other mutator.
  - `PhysicsWorld::set_boost_used_per_second(rate)` sets it for every car,
    present and later.
  - Default is `drive::BOOST_USED_PER_SECOND` (33.3); 0 is unlimited;
    negative counts as 0.
  - Each car's `DriveState` carries it to `apply_boost`.
- **Detection: from the capture.** `rb_verify_cli::boost_is_unlimited`
  returns true when both hold:
  - some car holds boost, with fuel in the tank, for at least 12 frames
    (0.1 s);
  - no car's fuel ever drops.

  `seed` then sets the rate to 0. A capture that never boosts can't tell,
  and keeps the default; nothing downstream depends on it there. A flag
  would be one more thing to get wrong per capture.

## Consequences

- `test2` free run: mean car position error 906 uu (was 1,365). The run is
  still chaotic after the kickoff, so its velocity error moves from 280 to
  390 uu/s.
- k-step and one-step are unchanged, since they already used the recorded
  fuel.
- A capture with boost pads, where fuel drops and refills, is detected as
  limited, which is correct. A capture with unlimited boost that holds
  boost for under 0.1 s is detected as limited, which costs at most 0.1 s
  of drain.
