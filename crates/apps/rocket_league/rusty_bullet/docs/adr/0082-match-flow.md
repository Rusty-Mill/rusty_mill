# ADR-0082: Match flow lives in `rb_env`, with the game's measured phases

- Status: Accepted
- Date: 2026-10-08
- Deciders: baileyrd
- Related: `RB-PHYSICS-001-FR-160`, FR-151 (respawn), PARITY-PLAN workstream E, `RB-RESEARCH-O024`

## Context

The port had physics only; a goal did nothing. The owner chose match flow next. The tape rig runs
freeplay, which has no phases, so a probe (`rb_match_log`) started a real match with one Psyonix
bot per team and logged every packet (phase, score, clock, ball, cars, pad count) over 16 goals.

## Decision

- `rb_env::flow::Flow` is a pure phase machine (goal line, phase lengths, touch test); `Env` does what a
  phase asks of the world. It is off by default (`enable_match_flow`), so existing users and every
  recording comparison are unchanged.
- `PhysicsWorld::kickoff(slots)` reuses the respawn path (spawn points, third of a tank) and adds the
  centre-spot ball and the pad reset; `hold_cars_on_their_spots` and `pin_ball_to_centre_spot` keep the
  countdown still, as the game does.
- The phase lengths and the goal line are constants with their measurements in the doc comments.
- The random slot choice is an input (`set_kickoff_slots`) with a rotating default, as the respawn pick.

## Consequences

- A policy or a script can play whole points: score, replay, countdown, restart.
- Not covered: the match clock and overtime, the first kickoff's intro, the teams' drop-height stagger.
- The probe needs the game up as the tape runner does; its logs are not committed.
