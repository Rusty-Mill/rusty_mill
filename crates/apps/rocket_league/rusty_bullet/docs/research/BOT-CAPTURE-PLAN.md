# Scripted-bot capture plan

Plan for `RB-RESEARCH-O005`: use RLBot to run repeatable, scripted mechanics
in the real game while the existing BakkesMod plugin records them, so each
mechanic gets a clean ground-truth capture with known inputs. Status: plan
only; nothing built. Sources: [RLBot FAQ](https://rlbot.org/faq/),
[RLBot v5 scripts](https://wiki.rlbot.org/v5/botmaking/scripts/),
[RLBot v5 index](https://wiki.rlbot.org/v5/),
[RLBotTraining](https://pypi.org/project/rlbottraining),
[state setting (v4 wiki)](https://github.com/RLBot/RLBotPythonExample/wiki/Manipulating-Game-State).

## Why scripted, not skilled

For physics validation the bot needs to replay the same inputs from the
same start, not to play well. A scripted controller gives identical runs
(so run-to-run noise is measurable) and exact inputs; the community ML bots
(Necto, Nexto) play full matches and cannot be told to perform a named
mechanic.

## Pieces

1. **Scenario**: a small file naming the initial state (car position,
   velocity, rotation, spin, boost; ball state) and a per-tick input
   timeline `{tick, throttle, steer, pitch, yaw, roll, jump, boost,
   handbrake}`. Same fields as the `input` object in the capture format
   (ADR-0005). Plain JSON; one file per mechanic.
2. **RLBot script** (v5 scripts run beside a match, declared in a
   `script.toml`, and can manipulate game state) sets the initial state
   when the scenario starts. Exact class and call names to be confirmed
   from the RLBot python-interface repository (the v5 wiki pages did not
   show them).
3. **RLBot bot** replays the input timeline tick by tick: a deliberately
   dumb "tape player".
4. **BakkesMod plugin** (`bakkesmod-plugin/rusty_bullet_capture`, already
   built) records the session as JSON Lines. It hooks the game's own
   vehicle-input event, so it records the inputs the game actually applied.
5. **Scoring**: `rb-verify --self-onestep` and `--self-kstep` on the
   capture, as for the owner's captures.

## Risks to settle first (one spike each)

- Does BakkesMod load and record when the game is started by RLBot (its
  `-rlbot` flag disables online play)? People combine them, but I have not
  confirmed it for this plugin.
- Does RLBot v5 run on the owner's current game build and launcher (Epic or
  Steam)? RLBot says PC only, Epic and Steam.
- Does state setting land on the tick we expect, and does the first
  captured frame equal the set state? Needed because `rb-verify` seeds its
  simulation from the capture's first grounded, neutral frame
  (`seed()` in `crates/rb_verify_cli/src/lib.rs`); an airborne start has
  none, so a `--seed-first-frame` option is needed, a small change.
- Determinism: run each scenario twice and diff; the difference is the
  noise floor for every later score.
- Terms: RLBot may be used offline only; no online play, no ranked.

## Mechanics, in order

| # | Scenario | Answers |
|---|---|---|
| 1 | Jump, wait 1.5 s, dodge; and drive off an edge, wait, dodge | the 1.25 s second-jump window and the never-jumped case (FR-136) |
| 2 | Wavedash: jump, pitch up, land and flip within a few ticks | landing with a flip, tire grip, wheel pushback |
| 3 | Pogo: drop the car nose-first from height, jump on contact, repeat | hard landings (the 18.35 s residual), jump re-arm on contact |
| 4 | Speed flip: kickoff diagonal flip, cancel, boost | flip torque, air roll, boost |
| 5 | Half flip, stall, flip cancel | flip clock and cancel rule |
| 6 | Hard landings and wall or corner slides from set velocities | the 9.125 s and 18.350 s residuals |
| 7 | Ball under the wheels (state-set ball) and flip reset | wheel-ball contact, jump refill (77.667 s) |
| 8 | Ball-contact mechanics: dribble, flick | car-ball friction and hit impulse |

Rows 1 to 3 and 6 target open questions already recorded in
`docs/PROJECT-STATUS.md`.

## Not doing

- ML policies for any of the above; revisit only for mechanics that need
  decisions (see `RB-RESEARCH-O005`).
- A reusable harness. Same stance as ADR-0005: a one-off script per spike
  until a second use shows it is needed.

## Next

Run risk spikes 1 and 2 on the owner's machine with a trivial tape (jump
once), capture it, and run `rb-verify` on the result.
