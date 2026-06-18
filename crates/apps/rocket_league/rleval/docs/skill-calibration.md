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

## What it fits (and the honest limit)

A detector's metric is only observed for events that **already passed** its gate,
so a hard minimum gate **cannot be lowered** from observed data (selection bias).
The calibrator therefore refits only the explicit confidence-ramp *upper* anchors
to a population percentile, leaving hard floors at their defaults:

- `high_aerial_height` ← corpus **p90** of observed aerial peak heights (kept clear
  of `aerial_min_height`), so the aerial confidence ramp spans the population.

Skills with fewer than 8 observations keep their default (a handful of replays
shouldn't move a threshold). The fitted config stamps `version =
"{base}-fitted"`.

**Follow-up:** to fit *floors* too, a "candidate-mode" detector would need to emit
each event's metric *before* the gate (so the sub-threshold tail is visible).
Until then, `refit_skill_config` (`skills/src/calibrate.rs`) only touches the
upper anchors — extend its match as the config gains more ramp-top fields.
