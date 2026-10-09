# ADR-0056: A shared scenario crate and `rb-verify --scenario`

- Status: Accepted
- Date: 2026-10-05
- Deciders: baileyrd
- Related: RB-VERIFY-003-FR-010, RB-RESEARCH-O005, ADR-0005

## Context

`tools/rb_tape_bot` replays a scripted scenario (initial state plus an input
tape) in the real game so the BakkesMod plugin can record it. We also want
the port's predicted trajectory for the same scenario, to compare with the
capture and to see what a scenario does before anyone runs the game. The
scenario reader lived in the tape bot, a standalone package that must not
join the workspace (its `rlbot` dependency stays out of CI).

## Decision

- New workspace crate `rb_scenario` (depends on `rb_domain`, `serde`,
  `serde_json`): the scenario format, `input_at(tick)`, the RLBot
  `[pitch, yaw, roll]` to quaternion conversion, and `initial_frame()`.
  Two call sites (the tape bot and `rb-verify`) justify it.
- `tools/rb_tape_bot` depends on `rb_scenario` by path and loses its own
  library.
- `rb_verify_cli::simulate_scenario` and `rb-verify --scenario <file>
  [every]` run a scenario through `rb_physics_bullet`.

Alternatives considered: duplicating the reader in both places (drift);
putting the scenario code in `rb_verify_cli` (the tape bot cannot depend on
a binary-facing crate that pulls the whole workspace).

## Consequences

- The README predictions for the shipped scenarios are reproducible.
- The rotation convention is covered by a round trip against a recorded
  quaternion (`test2.jsonl` 18.308 s); the roll sign is still unconfirmed in
  the real game.
- `rb_scenario` does not model anything; it only starts and feeds the port.
