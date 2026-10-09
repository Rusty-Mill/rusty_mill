# ADR-0058: Scenarios cut from a capture, and seeding from the first frame

- Status: Accepted
- Date: 2026-10-06
- Deciders: baileyrd
- Related: RB-VERIFY-003-FR-012, RB-VERIFY-003-FR-013, ADR-0056, ADR-0057
- Supersedes/Superseded by: none

## Context

The shipped scenarios are hand-written approximations of moments in the
owner's recordings. To replay a human performance exactly in the game, the
tape bot needs a scenario whose start state and inputs are the recording's
own. And a tape-bot recording starts where the bot set the state, often in
the air, so `rb-verify`'s `--self*` modes, which seed from the first
grounded, neutral frame, refuse it.

## Decision drivers

- One command from a recording window to a scenario the bot can play.
- The scenario must read back exactly (`from_json` unchanged, ADR-0056).
- Keep `rb_physics_bullet` untouched; tooling only.
- Keep the default seeding behaviour for every existing capture.

## Considered options

1. Cut scenarios by hand from `--self-trace` output. Error-prone, and the
   rotation conversion is easy to get wrong.
2. A separate binary for the cut. A third CLI for the same crate's logic.
3. `rb-verify --scenario-from` on `rb_verify_cli`'s library, with the
   quaternion-to-rotator inverse and `to_json` in `rb_scenario`. Chosen.

For seeding: a per-mode flag after the capture argument (five parsers to
touch) versus one global flag before the mode. Chosen: the global flag,
first on the command line, threaded as a `SeedFrame` parameter through the
six library functions that seed.

## Decision

- `rb_scenario::quat_to_rotator`, the inverse of `rotator_to_quat` with the
  same roll sign; `Serialize` derives and `Scenario::to_json`, which leaves
  absent start fields out.
- `rb_verify_cli::scenario_from_capture(frames, from, to, name)`: first
  car's and ball's state at the window's first frame, inputs run-length
  encoded within 0.01, `settle_ticks` 0, and a 120 Hz check (mean interval
  within 5%). Tick 0's input is the first frame's own input.
- `rb_verify_cli::SeedFrame { FirstGroundedNeutral, First }`; `rb-verify
  --seed-first-frame <mode> ...` selects `First`.

## Consequences

### Positive

- A human performance can be replayed and scored with two commands.
- Airborne-start recordings are scoreable with the existing `--self*`
  modes.

### Negative / tradeoffs

- The six seeding functions gained a parameter; callers must choose.
- `SeedFrame::First` trusts the first frame's hidden jump state to be
  neutral. A scenario cut mid-dodge will seed wrong without warning.
- Whether tick 0 should take the first frame's input or the next frame's is
  settled only by the acceptance run on `test2.jsonl`, which was not
  possible on the implementing machine (`replays/` absent). `FR-011`
  tolerates a one-tick shift, so the symptom would be lag, not mismatches.

## Validation and revisit triggers

- Run `rb-verify --scenario-from replays/test2.jsonl 8.95 9.2 > cut.json`
  then `rb-verify --scenario cut.json --against replays/test2.jsonl`;
  expect under 5 uu for the first 20 ticks and zero input mismatches, and
  record the numbers in `RB-VERIFY-003-FR-012`. If the lag is 1 tick, move
  the tape one frame later.
- Revisit if a real tape-bot capture is not 120 Hz within 5%.
