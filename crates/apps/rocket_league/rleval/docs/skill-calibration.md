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
candidate event. Each candidate **holds the detector's context gates** (what makes
the event an attempt at that mechanic) and **drops only the magnitude floor** being
fit, so the calibrator sees the full sub-threshold tail:

| Skill | candidate (`detect::*_candidates`) | held context | fitted floor |
|---|---|---|---|
| Aerial | car height at every touch | (a touch) | `aerial_min_height` |
| Flick | up-velocity of every *carried* ball | low + slow-incoming ball | `flick_min_up_dv` |
| Power shot | speed of every *goalward* touch | ball sent toward goal | `power_shot_min_speed` |
| Redirect | turn angle of every fast-in/out goalward touch | fast in, fast + goalward out | `redirect_min_angle_deg` |

The harness pools these per skill and prints a `└ candidates` line (count +
pre-gate p50/p90/p99). A skill where **nothing cleared the gate** still shows its
candidate line (e.g. `power_shot  0  none cleared the gate`) — the telling case that
a floor may be too high.

**Floor fit — the valley (`calibrate::fit_floor_valley`).** Each floor is set to the
midpoint of the largest gap between consecutive candidate values whose midpoint
falls in a per-skill **band** (`*_FLOOR_BAND`, bracketing the hand-set default) —
the valley between weak attempts and the real mechanic. A *percentile* of all
candidates would track how *often* players do the mechanic (the very thing we
study), so we fit the structural gap instead. The fit is **self-guarding**: the gap
must span at least `MIN_VALLEY_FRAC` (15%) of the band, else there is no real valley
(a continuous, unimodal distribution) and the **hand-set default is kept**. Aerial
height is genuinely bimodal (on-ground vs airborne) so it fits readily; the
continuous metrics (up-velocity, speed, angle) only move on a clear separation.

**Ramp tops.** Only aerials have a configurable top: `high_aerial_height` ←
candidate **p99.5** (kept ≥ floor + 100; falls back to the gated **p90** when no
aerial candidates). The flick/power/redirect ramps use fixed offsets from their
floor, so candidate-mode fits them **floor-only**.

Skills with fewer than 8 candidates (or observations) keep their default (a handful
of replays shouldn't move a gate). The fitted config stamps `version =
"{base}-fitted"`.

**Extending to more detectors:** factor a detector's per-candidate read out of its
gated body (as `flicks` shares `flick_candidate`), emit it from a `*_candidates`
twin, add it to `candidate_metrics`, and add a `fit_floor_into` call with a band to
`refit_skill_config` — the duration-gated runs (supersonic, ceiling, dribble) follow
the same shape.
