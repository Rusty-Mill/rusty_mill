# Skill-threshold calibration

`SkillConfig` (`skills/src/config.rs`) ships *pre-calibration* geometry guesses.
The `calibrate-skills` harness (`skills/src/bin/calibrate_skills.rs`, pure helpers
in `skills/src/calibrate.rs`) turns the labeled corpus into (a) a per-skill
**validation report** and (b) a **fitted config**. It mirrors the scoring
calibrator (`scoring/src/bin/calibrate.rs`).

## Run

```sh
# fetch the gitignored corpus first (needs BALLCHASING_API_KEY; GET only)
python assets/corpus/refresh_corpus_replays.py

# detect across the corpus, report, and write assets/corpus/fitted_skill_config.json
cargo run --release -p replay-skills --bin calibrate_skills        # default manifest
cargo run --release -p replay-skills --bin calibrate_skills <manifest.json>

# adopt the fit
replay-skills <match.replay> --config assets/corpus/fitted_skill_config.json
```

Corpus replays are gitignored, so a corpus-less checkout runs but reports little
(missing files are skipped); only ranked-doubles 2v2 entries are admitted.

## What it reports (the headline)

Per skill: instance `count`, the observed metric distribution `p10/p50/p90` (the
evidence magnitude in its natural unit — aerial peak height uu, power-shot ball
speed uu/s, redirect angle deg, …), and **`rho(rank)`** = Spearman between a
player's *mean* metric for that skill and their rank tier. A positive `rho` is the
detector-precision signal: better-ranked players really do pull bigger aerials /
faster shots. This validation is the primary value.

## What it fits

A *gated* metric is observed only for events that **already passed** the detector's
floor, so a hard minimum **cannot be lowered** from it (selection bias).

### Candidate-mode (fitting floors)

**Candidate-mode** removes that blind spot: `replay_skills::candidate_metrics`
(`skills/src/lib.rs`) has each detector also emit the pre-gate metric for *every*
candidate event, so the calibrator sees the full distribution — sub-threshold tail
included — and can place the floor as well as the top. The harness pools these per
skill and prints a `└ candidates` line under each skill's gated row (count +
pre-gate p50/p90/p99), so you can see how far below the floor the candidate
population sits.

Worked example — **aerials** (candidate metric = car height at *every* ball touch,
`detect::aerial_candidates`):

- `aerial_min_height` (floor) ← the **valley** between the dense ground-touch
  cluster (~17uu) and the aerial tail: the midpoint of the largest gap in candidate
  heights whose midpoint falls in the plausible band `[100, 600]` uu
  (`calibrate::fit_aerial_floor`). A *percentile* of all touches would track how
  often players aerial — the very thing we study — so we fit the structural gap
  instead. No clear valley ⇒ the floor is left at its default.
- `high_aerial_height` (top) ← candidate **p99.5**, kept ≥ floor + 100.

Without candidate-mode data the fit falls back to the previous behavior: only the
top moves, to the corpus **p90** of gated aerial peak heights, and the floor is
left untouched.

Skills with fewer than 8 candidates (or observations) keep their default (a handful
of replays shouldn't move a gate). The fitted config stamps `version =
"{base}-fitted"`.

**Extending to other detectors:** factor a detector's per-candidate metric out of
its gated body (as `aerial_touch` shares `touch_car_z`), emit it from a
`*_candidates` twin, add it to `candidate_metrics`, and add a floor fit to
`refit_skill_config`'s match — flicks (pop speed), power shots (ball speed),
redirects (angle) and the duration-gated runs follow the same shape.
