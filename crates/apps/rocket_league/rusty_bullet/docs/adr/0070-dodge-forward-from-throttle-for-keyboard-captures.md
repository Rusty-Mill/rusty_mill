# ADR-0070: The dodge's forward part follows the throttle only for keyboard captures

- Status: Accepted (validated against the game on 2026-10-07)
- Date: 2026-10-07
- Deciders: baileyrd
- Related: `RB-PHYSICS-001-FR-147`, `FR-103`, ADR-0023, `FR-072`, `FR-073`
- Supersedes/Superseded by: amends ADR-0023 (the rule stays, behind a flag)

## Context

ADR-0023 made a dodge with the pitch stick centred go forward as the throttle
says, because the owner's keyboard recordings do so and do not record the game's
own `DodgeForward` input. A fuzz tape (`fuzz_23`, a bot) shows the opposite for
a controller-driven car: with yaw 1, pitch 0 and throttle 1 the impulse is purely
sideways (842 uu/s = 500 x (1 + 0.9 x speed/2300) along the car's flat right), the
RocketSim rule `dodgeDir = (-pitch, yaw + roll)`. The port added a 363 uu/s forward
part: a 431 uu/s velocity error from one press.

## Decision drivers

- The engine's default is the game's rule, as bots (`rb_env` callers) use it.
- The owner's keyboard recordings must keep scoring as they do (their gate
  is not on every machine).
- No change to the public `ControllerInput`.

## Considered options

1. Drop the throttle rule (breaks the keyboard captures' 15 dodges).
2. Add `DodgeForward`/`DodgeStrafe` to `ControllerInput` and the capture format
   (a public API and format change, a plugin release).
3. A per-world flag, off by default, on in `rb-verify`'s capture modes.

## Decision

Option 3. `PhysicsWorld` and `Env` carry `dodge_forward_from_throttle`; the
`--self*` and `--sweep-hit` modes (keyboard captures) set it, `--scenario` (bot
recordings) does not.

## Consequences

### Positive

- A bot's side or diagonal dodge matches the game at speed (`dodgeth_yaw` 0.4,
  `dodgeth_pitch` 1.2 uu).
- The keyboard rule is kept where its evidence is.

### Negative / tradeoffs

- A caller that replays keyboard captures must set the flag; nothing guesses it.
- Option 2 would remove the flag (the plugin recording `DodgeForward`) and is
  the cleaner end state.

## Validation and revisit triggers

- The `drive` tests and the golden `dodgeth_*` fixtures.
- Revisit when the capture plugin records `DodgeForward`/`DodgeStrafe`.
