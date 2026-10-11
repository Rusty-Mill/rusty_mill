# RLEval evaluator: gap analysis and first milestone

Status: **design only, no code.** Written 2026-10-11.

## Goal

An evaluator of our own: learn what separates players at each rank from the rank-stratified
replay corpus, then take one player's replays and say concretely what to learn to reach the next
rank, with every suggestion citing a measured gap. The Spire capture (mitmproxy, OBS, 7-day
trial) is competitive research that informs this; nothing here depends on it.

This is a gap analysis. Most of the pieces exist (next section). It covers only what is missing,
and defines a first milestone that needs no simulator, using replay-level stats only. Paths are
under `crates/apps/rocket_league/` unless stated.

## What exists

| Capability | Where | Evidence |
|---|---|---|
| Decode a replay into a neutral match model | `crates/replay-analyzer` (boxcars adapter behind a `ReplayParser` port) | pure consumers downstream |
| 12-skill mechanics detection, kinematics only | `crates/replay-skills` | `rleval/docs/skill-detection.md`, `skill-calibration.md` |
| Decision-discipline scoring (positioning, rotation, boost, pace, ...) | `crates/replay-scoring` | composite tracks rank, cross-validated Spearman 0.809 on the full corpus |
| In-match value model (ΔV) | `crates/replay-value` | held-out AUC 0.730 (GBT) |
| Rank-stratified corpus: 1,055 replays processed, 952 clean, 3,432 in-bracket player-observations | `rleval/docs/rank-assessment-*.md` (Bronze to GC) | lobby-completeness gate, per-player rank verified against ballchasing tiers |
| Per-bracket distributions | `RankNorms` in `crates/replay-scoring/src/relative.rs`: per bracket, a 21-point percentile sketch (p0, p5, ..., p100) of the composite and of every metric | `rank_norms.json` written by the calibrate harness |
| Rank-relative report | `relativize` in the same file: per metric `within_rank_pct`, `bracket_median`, **`next_median`**, plus `rank_relative_leak` and `rank_relative_strength` | |
| Per-account history and habits | `crates/rleval-app/src/history.rs`: loss habits, recurring faults, trend line | `HabitReport`, `Focus` |
| "What to practice" per bracket | table in `rank-assessment-synthesis.md` | static, written by hand from the series |
| Which metrics exist and where they are shown | `rleval/docs/metrics-availability-audit.md` | as of 2026-06-27 |

Correction to an earlier summary in this project: the rank-relative layer already carries the
**next bracket's median** for each metric, so the gap-to-next-rank numbers exist in the data
model today. What does not exist is a generator that turns them into explainable suggestions.

## The five parts of the brief

### 1. Features extractable from replays

Exists: scoring metrics (boost management, reverse driving, aerial presence, pace, facing the
ball, second-man, spacing, recovery speed, ...), the 12 detected skills, per-player stat blocks,
and ΔV. The series reports mechanics presence per bracket as the share of players with at least
one occurrence per match.

Depends on recovered inputs (task 8, input recovery from replays): dodge direction and timing,
air-roll use, flip-cancel and input efficiency, steering-correction quality, boost-pulse
patterns. Today's detector infers mechanics from kinematics, so these are upgrades, not
prerequisites. Nothing in milestone 1 needs them.

Gap: the audit lists features as available, shown or neither; deciding which are *suggestable*
(next section) is new work.

### 2. Rank-tier profiles from distributions

Exists, and it is distributions rather than a black box: `RankNorms` holds per-bracket,
per-metric percentile sketches with the bracket's mean rank-tier id and sample count, built
from the labelled corpus. The synthesis documents each metric's path up the ladder.

Gaps and limits, all recorded by the series itself:
- **Not every metric climbs.** There are U-curves and reversals (the synthesis's own section),
  and above Champion the mid-ladder template reads deliberate elite style as a leak.
- **The rubric reads rank, not winning.** On the v14 run the composite tracks rank at about
  +0.81 but per-player ΔV at only +0.14. The decision-quality metrics (challenge timing,
  first-touch value, possession retention, recovery speed, transition readiness) sit within
  ±0.07 of zero against rank.
- **Within-bracket correlations are range-restricted.** The reliable signal is a bracket's
  position on the corpus-wide curves.

Consequence for suggestions: a metric may drive a "to reach the next rank" suggestion only if
it passes a **climb test**: its bracket medians rise (or fall, for lower-is-better) monotonically
across at least the brackets around the player's, and its between-bracket step is large against
its within-bracket spread. Everything else may still appear as a style or winning note, labelled
as such, never as a rank-up lever.

### 3. Gap analysis and suggestion generator

Exists: the raw gap (`raw` against `bracket_median` and `next_median`) and the leak and strength
picks, which are relative to the player's own bracket and weighted. The per-bracket practice
table is hand-written and static. `history.rs` finds loss habits over one account's matches.

Gap: the explainable generator. Proposed behaviour, a pure function over
`(RelativeReport, RankNorms, climb test, evidence)`:
1. Candidate set: non-experimental metrics that pass the climb test and have a computable raw
   and a `next_median`.
2. Score each by the gap to the next bracket measured in **bracket steps** (how many typical
   bracket-to-bracket steps the player lags), not raw units, so metrics are comparable.
3. Emit at most three suggestions. Each carries its evidence as data, not prose: the metric, the
   player's value, the own-bracket median, the next-bracket median, the percentile, the sample
   size behind both medians, and the climb-test result. The text is rendered from a fixed
   table mapping metric to a practice action, so no suggestion exists without a number behind it.
4. A single replay is noisy. With fewer than a set number of matches (open question below) the
   report shows the suggestion as provisional; with more, it uses the aggregate across matches
   from `history.rs`.

### 4. Evaluation: how we will know suggestions are right

What the corpus can show, and it should be done:
- **Stability on held-out replays.** Split by lobby, rebuild `RankNorms` on one half, and check on
  the other that the bracket medians keep their order and that suggestion sets agree in sign.
- **Per-tier calibration.** For held-out players, the "percentile in the next bracket up"
  should rise with rank. This is the existing composite validation, repeated per metric.
- **A suggestion-level check.** On held-out players, the metrics a suggestion names should be
  the ones on which that player lags the next bracket's median, by construction, so the more
  useful test is that the *same player's* suggestions are stable across their own replays.

What it cannot show: that closing a gap raises rank. The corpus is cross-sectional (different
players at different ranks), so every claim here is descriptive: "players one bracket up
typically have X". Causal evidence needs a player's later replays after a rank change. The 35
own replays are the only trend case in hand; ballchasing uploads by the same players over time
could add more, subject to the hygiene gaps below. The report wording must state the
descriptive claim, not a promise.

### 5. Data hygiene

Exists: a lobby-completeness gate (all four players present and active; leavers and AFKs drop the
whole lobby to low confidence), per-player rank verification against ballchasing tiers with
out-of-bracket lobby-mates excluded person by person, ranked 2v2 only.

To check, none confirmed from the repository:
- **Rank at match time versus at fetch time.** If the label is read when the replay is fetched
  it can differ from the rank at the time played.
- **Party play and smurfs.** No signal today. A smurf is labelled low and plays high, which
  inflates a low bracket's top end. Cheap mitigation: flag players whose composite is an extreme
  outlier for their label, and report how much they move each bracket's medians when removed.
- **Own 35 replays.** Identity is the platform id when present and the display name otherwise
  (`history.rs`); older name-only sessions are a separate identity. Check that all 35 resolve to
  one key.
- Replays stay in the private repository; no replay or per-player data enters git.

## Milestone 1: next-rank gap report (no simulator)

Input: one match's `Report` and a `rank_norms.json` (a corpus artifact under `rleval/assets/corpus/`, which is gitignored here and lives with the private corpus, so it is regenerated by the calibrate harness, not read from the repository). Output: up to three suggestions
as described in part 3, as JSON plus the existing report surface. No new data, no parser change,
no simulator, no recovered inputs.

Slices:
1. The climb test as a pure function over `RankNorms`, with its result per metric written into
   the artifact or computed at load. Tests on synthetic norms: monotone, flat, U-shaped,
   lower-is-better.
2. The generator: gap in bracket steps, selection, evidence record. Golden tests on a fixed
   `RankNorms` and a fixed report.
3. The metric-to-action table and rendering, with a test that every suggestable metric has an
   entry and every rendered suggestion contains its numbers.
4. Wire it into the report, and into the aggregate view from `history.rs` for the 35 replays.

Acceptance: for a replay at a known rank, each suggestion's cited numbers can be read back from
`rank_norms.json` by hand; no suggestion appears for a metric that fails the climb test; tests use a small synthetic `RankNorms`, never the private artifact; the top
bracket produces none (there is no next median).

## Later milestones

2. Aggregate suggestions over several replays with an interval, using `history.rs`.
3. Held-out stability and calibration reports (part 4).
4. Input-dependent features once task 8 delivers recovered inputs.
5. Longitudinal validation on the 35 replays, then on any same-player series that can be built.

## Risks

- **Over-claiming.** Descriptive gaps read as prescriptions. Wording and the climb test are the
  defence.
- **Top-bracket validity.** The series says the absolute rubric stops describing the game at GC;
  suggestions for Champion and above lean on the relative layer and ΔV and should be fewer.
- **Stale norms.** `rank_norms.json` carries a version stamp; a game update that changes physics
  or metrics invalidates the medians. Refit before trusting suggestions after one.
- **Sample size per bracket.** Brackets under the minimum are dropped by `RankNorms::build`;
  the top and bottom may be thin.

## Open questions

1. How many matches make a suggestion non-provisional? Needs the within-player match-to-match
   variance, which the corpus may not give if it has few replays per player.
2. Is the rank label read at match time or at fetch time (part 5)?
3. Should suggestions be framed against the next bracket or the next division within one?
   `RankNorms` buckets are whole brackets today.
4. Which metrics, if any, are worth a stricter climb threshold than the others?
