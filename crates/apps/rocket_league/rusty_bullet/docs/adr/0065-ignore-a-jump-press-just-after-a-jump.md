# ADR-0065: A jump press just after a jump is ignored

- Status: Accepted (validated against the game on 2026-10-07)
- Date: 2026-10-07
- Deciders: baileyrd
- Related: RB-PHYSICS-001-FR-139, FR-095, FR-136, RB-RESEARCH-O005,
  ADR-0064, `docs/research/BOT-RUN-SHEET.md` (session 2)
- Supersedes/Superseded by: none

## Context

The unattended tape runs (ADR-0064) made `speed_flip` repeatable, and the
port was 87.4 uu mean off the game on it, 273.7 uu at worst. The recorded
inputs (plugin 1.4 records the hooked input) show the dodge press reached
the game on tick 6 after the first jump press, and the game did nothing: no
dodge, no double jump, velocity continuing smoothly. The port flipped.

## Decision drivers

- A measured rule, not a guess: the tape bot can run any gap in seconds.
- One-step lag already established for air control (FR-095).
- No change to anything the existing captures agree on.

## Considered options

1. Special-case six ticks.
2. Gate a ground press on the jump record and an airborne press on the
   previous step's ground state (the existing lag), then check against a
   sweep.

## Decision

Option 2, measured first. A sweep of 28 tapes (first jump held 1, 3, 6, 12
ticks; released 1, 2, 3, 4, 6, 9, 15; then a one-tick forward-dodge press)
shows the game's rule: a press at most 6 ticks after the first press is
ignored, 7 or more flips. Implementation: a ground press is skipped while
`JumpClock::has_jumped`; an airborne press needs `!was_on_ground`.

## Consequences

### Positive

- `speed_flip` 87.4 / 273.7 -> 4.8 / 16.6 uu (mean / max); all 28 sweep
  outcomes match; the six ignored-press tapes 54 -> under 2.2 uu.
- Other tape-bot scenarios unchanged.

### Negative / tradeoffs

- A press the first step after the wheels leave the ground now does nothing
  anywhere (a ramp take-off, a landing bounce). The captures that touch it
  agree; the owner's recordings could not be re-run on this machine.
- The sweep used a forward dodge from a standing start; other stick
  directions and speeds are assumed alike.

## Validation and revisit triggers

- Test `a_press_just_after_a_jump_is_ignored_until_the_car_is_airborne`.
- Re-run the `test2`/`hitjump`/`front`/`side` k = 30 gate where those
  recordings exist; revisit if any worsens.
- Revisit if a capture shows a flip or double jump on the first airborne
  step (a jump off a ramp or wall).
