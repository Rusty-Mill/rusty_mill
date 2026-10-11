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
| Rank-stratified corpus: 1,055 replays processed, 952 clean, 3,432 in-bracket player-observations (the historical analysis; the norms generator filters differently, see part 2) | `rleval/docs/rank-assessment-*.md` (Bronze to GC) | lobby-completeness gate, per-player rank verified against ballchasing tiers |
| Per-bracket distributions | `RankNorms` in `crates/replay-scoring/src/relative.rs`: per bracket, a 21-point percentile sketch (p0, p5, ..., p100) of the composite and of every metric | `rank_norms.json` written by `bin/calibrate.rs` from all scored samples. `BucketNorm.n` is a bucket-wide observation count; the metric sketches carry no per-metric counts |
| Rank-relative report | `relativize` in the same file: per metric `within_rank_pct`, `bracket_median`, **`next_median`**, plus `rank_relative_leak` and `rank_relative_strength` | |
| Per-account history and habits | `crates/rleval-app/src/history.rs`: loss habits, recurring faults, trend line | `HabitReport`, `Focus` |
| **Shipped plan generator** | `crates/rleval-app/src/progress.rs`, served at `/api/history/progress` and shown in the app UI. Groups a player's matches into play sessions (2 h gap), needs at least 3 matches per metric (`MIN_MATCHES`), picks the 3 metrics (`PLAN_LEN`) with the lowest mean within-bracket percentile, sets the target to the mean next-bracket median (the own-bracket median when there is no next bracket), and reports whether the latest session moved toward it | `progress()`, `PlanItem`, `Trend` |
| "What to practice" per bracket | table in `rank-assessment-synthesis.md` | static, written by hand from the series |
| Which metrics exist and where they are shown | `rleval/docs/metrics-availability-audit.md` | as of 2026-06-27 |

Corrections to earlier summaries in this project: the rank-relative layer already carries the
**next bracket's median** for each metric, and a **plan generator already ships**
(`progress.rs`, in the app and its UI). What does not exist is the part this note asks for:
suggestions that pass a climb test, carry their evidence as data, are ordered by gap to the next
bracket, handle the top bracket, and rest on norms whose provenance and per-metric counts are
recorded.

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
per-metric percentile sketches with the bracket's mean rank-tier id and a bucket-wide count,
built from the labelled corpus. The synthesis documents each metric's path up the ladder.

**Provenance caveat.** The figures in the series (952 clean replays, 3,432 observations, the
Spearman values) come from its own filtered analysis. `bin/calibrate.rs` builds `RankNorms` from
*all* scored samples after assigning each replay's bucket to its rank-joined players; the
confidence-filtered list there is used for diagnostics only. So the clean-corpus numbers describe
the analysis, not necessarily the samples behind the shipped norms. The private artifact was not
inspected for this note. A norms generator for suggestions must state its filter (confidence,
lobby completeness, rank membership) and record it in the artifact's version before the
medians are trusted.

**Sufficiency caveat.** `BucketNorm.n` counts the bucket's observations, but a metric's sketch
omits missing and non-finite values and keeps no count of what it used, so `n` cannot show how
many observations stand behind any one median. This needs a schema change (a count per metric) and
a calibration change to write it, plus a minimum per-metric count below which a metric is not
suggestable. No recalibration is proposed or authorized here.

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

Exists: the raw gap (`raw` against `bracket_median` and `next_median`), the leak and strength
picks, `history.rs` loss habits, and the shipped plan in `progress.rs` described in the table
(three targets by lowest within-bracket percentile, at least three matches, next-bracket median as
the target, a trend across sessions). The per-bracket practice table is hand-written and static.

Gap: the proposal is to **extend or replace the selection inside `progress.rs`**, keeping its
session grouping, minimum-match rule and trend, not to build a second path. What changes:
ordering by lowest within-bracket percentile becomes ordering by gap to the next bracket in bracket
steps; a climb test filters candidates; each item carries its evidence; the top bracket is handled
explicitly instead of targeting its own median; and per-metric sufficiency replaces reliance on a
bucket-wide count. Proposed behaviour, a pure function over the player's match snapshots, the
norms, the climb test and the evidence:
1. Candidate set: non-experimental metrics that pass the climb test, meet the per-metric
   minimum count, and have a computable raw and a `next_median`.
2. Score each by the gap to the next bracket measured in **bracket steps** (how many typical
   bracket-to-bracket steps the player lags), not raw units, so metrics are comparable.
3. Emit at most three suggestions. Each carries its evidence as data, not prose: the metric, the
   player's value, the own-bracket median, the next-bracket median, the percentile, the
   per-metric sample sizes behind both medians (the new schema field), and the climb-test result. The text is rendered from a fixed
   table mapping metric to a practice action, so no suggestion exists without a number behind it.
4. A single replay is noisy. `progress.rs` already requires three matches per metric; the open
   question below is whether three is the right threshold, and whether fewer should show as
   provisional instead of being hidden.

### 4. Evaluation: how we will know suggestions are right

What the corpus can show, and it should be done:
- **Stability on held-out replays.** Split by lobby, rebuild `RankNorms` on one half, and check on
  the other that the bracket medians keep their order and that suggestion sets agree in sign.
- **Per-tier calibration against a fixed reference.** A player's percentile within their *own*
  next bracket need not rise with rank: if brackets are equally spaced with equal spread, every
  player sits at the same percentile whatever their rank. So place every held-out player's value
  on **one fixed reference distribution** (for example the pooled corpus) and check that it rises
  with rank, repeated per metric, as the existing composite validation does. Or validate each
  source bracket separately against its own fixed next bracket.
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

Input: a player's match snapshots (as `progress.rs` takes them) and a `rank_norms.json` (a corpus
artifact under `rleval/assets/corpus/`, gitignored here and living with the private corpus, so it
is regenerated by the calibrate harness, not read from the repository). Output: up to three
suggestions as described in part 3, as JSON plus the existing app surface. No parser change, no
simulator, no recovered inputs.

Milestone 1 is not purely code. It needs norms with per-metric counts and a recorded filter, which
means a calibration change and a regenerated private artifact. Writing the code and tests against
synthetic norms comes first; regenerating the artifact needs the owner's explicit approval and is
not authorized by this note.

Slices:
0. Norms provenance and sufficiency: the generator's filter stated and recorded in the artifact
   version; a per-metric count in the schema and in the calibrate output; the minimum count.
   Code and tests only; the artifact is regenerated later, with approval.
1. The climb test as a pure function over `RankNorms`, with its result per metric written into
   the artifact or computed at load. Tests on synthetic norms: monotone, flat, U-shaped,
   lower-is-better.
2. The selection change in `progress.rs`: gap in bracket steps, climb-test filter, evidence
   record, top-bracket behaviour. Golden tests on a fixed `RankNorms` and fixed snapshots,
   including the existing `progress` tests so session grouping and trends do not regress.
3. The metric-to-action table and rendering, with a test that every suggestable metric has an
   entry and every rendered suggestion contains its numbers.
4. The app and UI surfaces that already show the plan, updated to display the evidence; then
   run it on the 35 replays.

Acceptance: for a replay at a known rank, each suggestion's cited numbers can be read back from
`rank_norms.json` by hand, including its per-metric count; no suggestion appears for a metric that
fails the climb test or falls under the minimum count; tests use a small synthetic `RankNorms`,
never the private artifact; the top bracket produces no rank-up suggestion (there is no next
median) instead of targeting its own median.

## Later milestones

2. Noise handling beyond the three-match rule: an interval on each metric's mean, and
   provisional display below the threshold.
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
- **Sample size.** Brackets under the minimum are dropped by `RankNorms::build`, and the top and
  bottom may be thin; per metric the count is not recorded today (part 2).
- **Norms provenance.** The shipped norms may not match the filtered analysis the figures come
  from (part 2). Until the filter is recorded, treat suggestions as indicative.

## Open questions

1. How many matches make a suggestion non-provisional? Needs the within-player match-to-match
   variance, which the corpus may not give if it has few replays per player.
2. Is the rank label read at match time or at fetch time (part 5)?
3. Should suggestions be framed against the next bracket or the next division within one?
   `RankNorms` buckets are whole brackets today.
4. Which metrics, if any, are worth a stricter climb threshold than the others?
5. Which filter should the norms generator apply (confidence, lobby completeness, rank
   membership), and which did the shipped artifact use? Answering needs the private artifact.
6. What per-metric minimum count makes a median trustworthy enough to suggest from?
