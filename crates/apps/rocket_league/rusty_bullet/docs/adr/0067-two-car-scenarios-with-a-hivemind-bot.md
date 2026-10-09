# ADR-0067: Two-car scenarios driven by a hivemind bot

- Status: Accepted (validated against the game on 2026-10-07)
- Date: 2026-10-07
- Deciders: baileyrd
- Related: `RB-VERIFY-003-FR-018`, `RB-PHYSICS-001-FR-006`, ADR-0059, ADR-0064
- Supersedes/Superseded by: none

## Context

Every real recording so far has one car, so car-vs-car physics (bumps and
demolitions: `world.rs` says "Bumps and demolitions are not modeled") had no
ground truth. The tape bot plays one car's tape; two cars need two tapes on
one clock.

## Decision drivers

- A bump test needs both cars to start and move on the same tick; two bot
  processes each find their own "first ticking frame", a few ticks apart.
- No new dependency; the one-car path must not change.

## Considered options

1. Two bot processes, each indexing its tape from its own first frame.
2. Two bot processes synchronised through the game state (each waits for car 0
   to reach its start location).
3. One RLBot hivemind process driving every car of a team.

## Decision

Option 3. `Scenario` gains `others` (further cars, each with a start and its own
steps; `settle_ticks` shared), so a scenario file with no `others` is exactly
the old one-car format. `rb_tape_hive` (a second bot binary, sharing
`start_state` and `controller` with `rb_tape_bot` through a small library) sets
every car's and the ball's state on the first ticking packet and sends one
input per car per packet. `rb_run_tapes` sends core one player entry per car,
all of team 0 with the hivemind flag, when the scenario has more than one car.
`rb-verify --scenario ... --against` simulates every car (`Env` already steps
several), scores each (`car N error: mean .. max ..`), and the recorded-input
replay reads each car's own input.

## Consequences

### Positive

- Two-car recordings run unattended like the one-car ones: five bump probes
  worked on the first run.
- The first measurement of the game's bump: a 500 uu/s push on a stopped car
  sends it to 1293 uu/s and up into the air; the port gives it 496 uu/s
  (RB-RESEARCH-O018).

### Negative / tradeoffs

- A hivemind is one team, so these scenarios have teammates: bumps are
  measured, demolitions (opposing teams) are not yet.
- `rb_tape_bot` and `rb_tape_hive` duplicate the frame gate; a third consumer
  should move it into the library.

## Validation and revisit triggers

- `rb_scenario` and `rb_verify_cli` tests; five bump captures scored.
- Revisit for demolitions (a second bot or a script to change team).
