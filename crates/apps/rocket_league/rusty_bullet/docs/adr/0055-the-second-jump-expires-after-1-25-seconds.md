# ADR-0055: The second jump expires 1.25 s after the first jump ends

- Status: Accepted
- Date: 2026-10-05
- Deciders: baileyrd
- Related: RB-PHYSICS-001-FR-136, FR-094
- Supersedes/Superseded by: —

## Context

RocketSim's `_UpdateDoubleJumpOrFlip` allows a double jump or dodge only
while `airTimeSinceJump < DOUBLEJUMP_MAX_DELAY` (1.25 s). That timer runs in
the air only after a jump whose hold has ended, and is zero otherwise. The
port tracked only whether the second jump was available, so a late dodge
fired.

## Decision

- `jump::JumpClock { has_jumped, jump_time, air_time_since_jump }`, updated
  each tick before the jump press is handled. Past the window the port
  clears `double_jump_available`. Wall contact refills it with the second
  jump; landing forgets the jump after the reset window.

Alternatives considered: a single "airborne seconds" counter. Rejected: it
would expire a car that drove off an edge, which RocketSim does not.

## Consequences

- No recording has a second jump later than the window; every capture score
  is unchanged. One test covers inside, outside and never-jumped.
- Unverified against the real game for the never-jumped case; the rule is
  RocketSim's, to be checked by a capture of a late dodge.
