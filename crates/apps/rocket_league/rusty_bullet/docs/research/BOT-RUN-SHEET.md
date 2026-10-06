# Bot run sheet (first real run of `rb_tape_bot`)

One session on the machine with the game, RLBot v5 and BakkesMod. Setup and
risks: `tools/rb_tape_bot/README.md`. Plan: `BOT-CAPTURE-PLAN.md`. Nothing
here has run against the game yet; each stage below settles one open risk
before the next depends on it. Stop at the first failure and bring back the
output.

## Stage 0: build and load (10 min)

- `cargo build --release` in `tools/rb_tape_bot`; add `bot.toml` in the RLBot
  GUI; `plugin load rusty_bullet_capture`.
- Pass: the bot appears in an offline match and prints its tape length.

## Stage 1: the three risks, with the control scenario (15 min)

Run `prompt_dodge` (`RB_TAPE=scenarios\prompt_dodge.json`), capturing with
`rb_capture_start prompt_dodge.jsonl` / `rb_capture_stop`.

| Risk | Pass |
|---|---|
| BakkesMod records while RLBot runs | the file has frames |
| State setting from a bot works | first car frame is within 5 uu of the scenario `location` |
| 120 packets/s | `rb-verify --scenario scenarios/prompt_dodge.json --against replays/prompt_dodge.jsonl` reports lag 0 or 1, not a drifting error |

Also read the first frame's rotation against the scenario's (roll sign is the
likeliest wrong convention). Run it twice and diff to measure run-to-run
noise; that noise is the floor for every later comparison.

## Stage 2: the open questions (one run each, 5 min each)

| Scenario | Question it answers | Port predicts |
|---|---|---|
| `late_dodge` | is the second-jump window 1.25 s (FR-136)? | no dodge |
| `wavedash_early` / `_mid` / `_late` | landing with a flip | hop back up, then keep about 1350 uu/s |
| `speed_flip`, `half_flip` | flip, cancel and air roll together | see README table |
| `pogo`, `hard_landing_nose_first` | nose landings, jump on contact | see README table |
| `corner_slide`, `car_over_ball` | the recorded residuals, with exact inputs | see README table |

## Stage 3: your own windows (optional)

Cut any 0.25 to a few seconds from a capture and replay it exactly:

```
rb-verify --scenario-from replays/test2.jsonl 8.95 9.2 mine > scenarios/mine.json
```

(input fields and the 120 Hz check are validated; a 20 Hz capture is
refused). Acceptance seen on this cut: lag 0, position error under 2.5 uu,
zero input mismatches.

## What to bring back

For each run: the capture in `replays/` (gitignored), the
`--scenario ... --against` output, and anything the game did that looked
wrong (dropped inputs, a reset, a wrong spawn). Then the loop is: largest
error, diagnose, fix, record in the spec, as usual.
