# Pacifist score — corpus validation (v0 baseline → v2.3)

The first ground-truth run of the consolidated Pacifist analyzer
(`replay-pacifist`, ported in the PacifistScore consolidation): the three
implemented dimensions at their shipped defaults, scored across the full
seven-bucket ranked-2v2 corpus, rank-joined per player, gated by the same
lobby-completeness check the rank-assessment series used.

**Harness:** `cargo run --release -p replay-pacifist --features
corpus-validate --bin validate_pacifist` — mirrors the scoring crate's
`calibrate` shape and reuses its rank-join, Spearman, and coverage-gate
primitives. **3,548 rank-joined players** scored (100 replays gated for
leavers/AFK — consistent with the series' 101; 2 files deleted upstream).

## The result: the v0 score does not track rank

```
pacifist value vs tier : ρ = +0.010

per-dimension Spearman(value, rank)      n
  over-extension            +0.006    3548
  commitment-discipline     +0.012    3548
  boost-economy             -0.042    3548

per-bucket        n     mean    p50   within-bucket ρ
  bronze          90    70.9   71.3   +0.072
  silver          218   73.4   73.8   -0.085
  gold            416   74.4   74.4   +0.027
  platinum        444   75.4   75.3   +0.055
  diamond         680   74.6   74.6   +0.044
  champion        847   74.2   74.4   +0.030
  grand-champion  853   74.2   74.3   +0.072
```

Bracket means sit in a 4.5-point band (70.9–75.4) across a ladder that the
workspace's calibrated composite spreads over 40+ points (fitted per-rank
means 32.9 → 62.1 on the same corpus, CV ρ 0.809). A Bronze lobby and an SSL
lobby get essentially the same Pacifist discipline score. Every dimension is
individually flat.

## Why — the operationalization gap, not the concept

The *behaviors* these dimensions name demonstrably carry rank on this exact
corpus — the rank series measured their discrete-event analogues climbing
the whole ladder (`double_commit_rate` 4.9% → 57.9% of the calibrated range,
`overcommit_rate` 3.4% → 68.3%, `boost_starvation` −4.2% → 70.7%). So the
signal exists; this v0 *measurement* of it doesn't discriminate. Three
compounding causes, all visible in the design's own caveats:

1. **Per-frame fractions saturate where per-opportunity rates discriminate.**
   Each dimension scores `100·(1 − penalised_frames/relevant_frames)`. But
   the penalised conditions are near-invariant by frame count: the
   rank series found behind-ball share is ~70–75% at *every* bracket (a
   series invariant) — and "teammate goalside" / "2nd man goalside" are
   restatements of exactly that. The discriminating versions of these ideas
   in the scoring crate count discrete *events per opportunity* (commits per
   challenge window), not frames.
2. **The positional possession proxy barely fires.** `context.rs` labels
   its possession model "deliberately weak… a v1 placeholder, replaceable
   once touch decode lands" — on a real match it reads ~69% *contested*
   (the smoke run), so possession-conditional logic mostly never engages.
   Touch-decoded possession **already exists in this workspace**
   (`analyze::events` possession runs, the same model the ΔV track uses) —
   feeding it through the bridge is precisely the upgrade the original
   design anticipated.
3. **Uncalibrated thresholds.** `engage_radius_uu = 1200`,
   `empty_boost = 12`, `control_radius_uu = 350` are hand-set guesses (the
   design doc says so), and the boost-economy dimension comes out weakly
   *inverted* (−0.042) — consistent with the series' finding that higher
   ranks deliberately run leaner tanks, so "low boost near the ball" is
   partly *style* at the top, not error.

One philosophical note, stated and then set aside: the Pacifist score
measures *adherence to a specific system*, and the book's own framing never
promises rank correlation (its blueprints cap around Champ–GC). A perfect
adherence metric could legitimately de-correlate at the top — the rank
series' "textbook boundary" finding predicts exactly that. But that defense
does not cover *this* result: Bronze out-"disciplining" half the ladder and
a 4-point total spread mean the v0 extractors aren't yet measuring the
system, so the boundary question stays open until they do.

## What this buys the roadmap

This run is the baseline the next iteration is judged against, and it
re-orders the work:

1. **Replace the possession proxy with the canonical possession events**
   via the bridge (the designed-for upgrade; unlocks every
   possession-conditional criterion including T-1, the system's core rule).
2. **Rework the three dimensions from frame-fractions to per-opportunity
   event rates**, built from the criteria spec's concrete rows (F17
   last-man dive, F4/FM-2 zero-boost aggression, FM-1's Major/Minor
   counting) rather than frame shares of near-invariant conditions.
3. **Re-run this harness** — same command, same gate, same join — and
   require the reworked dimensions to beat this baseline before building
   the remaining five on top.

The harness itself is the durable artifact: any future dimension gets the
same 3,500-player, seven-bucket, leaver-gated evaluation for one command.

---

# v1 (`pcfg-v1`): per-opportunity event rates — re-run results

The rework the v0 diagnosis prescribed, re-measured with the identical
harness, gate, and join (same 3,548 rank-joined players):

- **Possession**: touch-decoded canonical possession runs fed through the
  bridge (`bridge::possession_spans` →
  `MatchContext::derive_with_possession`); the positional proxy remains only
  as the default for synthetic/domain-only callers. Contested share on real
  play drops ~69% → ~49%.
- **Dimensions**: rebuilt around a shared **engagement episode** primitive
  (1st man crossing into challenge range — hysteretic radii, entry-time
  facts). Over-extension = last-man commits against non-owned possession
  with no cover (F17/T-1); commitment discipline = the cover man joining
  the teammate's live engagement (F9/FM-1 double-commit counting); boost
  economy = engagements entered with an effectively empty tank (F4/FM-2).
  Confidence saturates on episodes, not frames.

```
pacifist value vs tier : ρ = +0.068         (v0: +0.010)

per-dimension Spearman(value, rank)      n        v0
  over-extension            -0.083    3548    +0.006
  commitment-discipline     +0.116    3548    +0.012
  boost-economy             +0.122    3548    -0.042

per-bucket        n     mean    p50   within-bucket ρ      v0 mean
  bronze          90    55.7   55.0   +0.051                 70.9
  silver          218   60.3   59.9   -0.091                 73.4
  gold            416   61.3   61.2   +0.113                 74.4
  platinum        444   63.3   63.5   -0.002                 75.4
  diamond         680   64.3   64.2   +0.053                 74.6
  champion        847   64.0   64.1   -0.072                 74.2
  grand-champion  853   62.0   62.1   -0.046                 74.2
```

## Reading the v1 result

1. **Every dimension moved in the diagnosed direction.** Boost economy's
   inversion is fixed (−0.042 → +0.122) — judging the tank *at the commit*
   instead of near-ball frame shares separates deliberate lean play from
   empty dives. Commitment discipline strengthened 10× (+0.012 → +0.116).
   Bracket-mean spread doubled (4.5 → 8.6 points) and gained structure.

2. **The structure is an inverted U, and that is the finding.** Means rise
   monotonically Bronze → Diamond (55.7 → 64.3), flatten at Champion, and
   *fall* at GC (62.0). This is exactly the "textbook boundary" the
   rank-assessment series measured from the other direction: inside GC,
   double-commits invert (+0.158 — coordinated double-pressure), spacing
   tightens, and deliberate last-man challenges are routine at SSL pace.
   `over-extension`'s negative overall ρ (−0.083) is that phenomenon, not a
   defect: the top of the ladder *does* commit as last man more, on
   purpose. A Pacifist-adherence score **should** peak where the book's own
   curriculum peaks — its rank blueprints stop at Champ–GC — and v1 now
   measures real behavior well enough to reproduce that shape from the
   data. Consequence for product use: surface the Pacifist score as
   *system adherence with a mid-ladder target audience*, not as a rank
   proxy; the overall-ladder ρ is structurally capped by the U-turn.

3. **Within-bracket ρ remains ≈ 0 everywhere** — one match yields only
   5–15 episodes per player per dimension, so single-match divisional
   resolution is likely beyond this instrument regardless of definition
   quality; multi-match aggregation (the service layer's per-account view)
   is the realistic path to stable per-player placement.

## Remaining work this baseline motivates

- **FM-1 severity model** *(shipped as `pcfg-v1.1` — next section)*: Major
  faults (the F17 last-man dive that concedes, FM-2's 0-boost corner flip)
  should gate/cap rather than average — the guide's 15-minor/1-major framing
  is a different aggregate shape than the current weighted mean.
- The **five unbuilt dimensions**, now on the episode primitive.
- **Threshold calibration** (enter/exit radii, double radius, empty-boost)
  against labeled "very Pacifist vs very not" replays, per the design doc —
  rank is deliberately *not* the calibration target given (2).

---

# v1.1 (`pcfg-v1.1`): the FM-1 severity model — re-run results

The guide's aggregate is not a weighted mean (criteria spec FM-1: *"up to 15
Minor faults; one Major fault = instant failure"*), so v1.1 adds a severity
ledger alongside the dimension averages, classified from the same engagement
episodes the v1 dimensions score:

- **Major** — the FM-2-shaped compound dive, all three conditions at the
  commit: last man (no teammate goalside) + a ball the team does not own +
  an empty tank. One Major fails the verdict and **caps the headline value
  at 40** (the cap is an invented threshold — the guide only says "instant
  failure" — and is documented as such in `SeverityConfig`).
- **Minor** — the single-condition faults: F17 (fueled last-man dive), F4
  (covered-but-empty engagement), F9 (double-commit). Minors accumulate
  toward the guide's allowance of 15 and flip the verdict past it, but
  deliberately do **not** touch the number — the dimension averages already
  price each penalised episode in, so subtracting again would double-count.

Identical harness, gate, and join (same 3,548 rank-joined players):

```
pacifist value vs tier : ρ = +0.069     (v1: +0.068 — the Major cap costs no rank signal)

FM-1 severity (Major/Minor faults, driving-test verdict)
  minors vs tier : ρ = −0.023
  majors vs tier : ρ = −0.121
  overall verdict: 224/3548 pass (6.3%)

per-bucket        n     mean    p50   within-ρ   minors  majors   %pass
  bronze          90    44.6   40.0    +0.095     33.7    2.13     5.6
  silver          218   49.7   40.0    −0.036     29.1    1.16    12.8
  gold            416   49.8   40.0    +0.062     30.3    1.14     6.0
  platinum        444   52.1   40.0    +0.077     27.9    0.82     8.3
  diamond         680   53.9   58.9    +0.071     28.1    0.72     6.2
  champion        847   53.1   53.9    −0.003     28.0    0.74     4.7
  grand-champion  853   52.0   50.9    −0.002     29.6    0.73     5.5
```

(The per-dimension rows are unchanged from v1 to the third decimal — the
severity pass reads the same episodes without touching the extractors, so
this is the expected consistency check.)

## Reading the v1.1 result

1. **The Major count is the strongest single severity signal the instrument
   has produced.** Majors-vs-tier ρ = −0.121 (more rank, fewer Majors), and
   the per-bucket means fall ~3× from Bronze to Diamond (2.13 → 0.72 per
   match) — then go *flat* through GC. The FM-2 compound dive is a
   low-ladder failure that bottoms out at Diamond; the residual ~0.7 per
   match above it is consistent with the rank series' finding that top
   brackets challenge as last man deliberately. Minors, by contrast, carry
   almost nothing (−0.023): as raw per-match counts they are dominated by
   how often a player engages at all, which is the exposure the dimension
   *rates* already normalize away.

2. **The driving-test verdict at guide strictness is a near-universal
   fail.** 6.3% of the ladder passes, `%pass` has no rank structure (Silver's
   12.8% is the maximum, Champion's 4.7% the minimum), and the average
   player carries ~28–34 minors — roughly double the allowance — so the
   verdict usually fails on minors alone before any Major lands. Two honest
   readings: as a *discriminator* the verdict is useless, and the small pass
   set is confounded with exposure (few episodes → few chances to fault);
   as a *product statement* it is exactly the guide's point — almost nobody
   on the ranked ladder plays inside the Pacifist system's tolerances for a
   full match. It belongs in the coaching read ("FAIL — 31 minors, 2
   majors"), not in a leaderboard.

3. **The cap compresses the bottom of the ladder onto 40.0 exactly.** The
   Bronze-through-Platinum medians sit *at* the cap — the median player
   there carries at least one Major — while Diamond and above escape it
   (medians 58.9 / 53.9 / 50.9). Despite that compression, the headline ρ
   is unchanged (+0.068 → +0.069) and the inverted U persists (peak at
   Diamond, dip at GC), so the severity shape adds the verdict and the
   fault ledger without costing the v1 baseline anything.

What remains is unchanged from v1: the five unbuilt dimensions on the
episode primitive, and threshold calibration against labeled-adherence
replays — now including `major_cap` and whether the 15-minor allowance
should scale with match length.

---

# v2 (`pcfg-v2`): the full eight-dimension rubric — re-run results

The five remaining dimensions, each built on the existing primitives with a
distinct opportunity type so none restates another:

- **positioning-fit** — the back man judged at the *opponent's* engagement
  entries: penalised if not goalside at their commit (the right area for the
  situation × role).
- **rotation-soundness** — the player's own episodes: penalised if they
  never recover goalside within 4 s of the episode ending (ball-chasing
  instead of rotating out).
- **challenge-timing** — the player's challenges: penalised when clearly
  beaten to the ball at the commit (estimated time-to-ball vs the best
  opponent, the design doc's named signal).
- **shadow-quality** — teammate episodes covered as 2nd man: penalised if
  goalside of the ball for less than half the episode (distinct from
  commitment discipline, which only punishes *joining*).
- **shot-selection** — the replay's per-player scoreboard shot events,
  newly bridged: penalised for long-range (>4000 uu) or wide-angle (>60°)
  hero shots.

Default weights follow the design doc's confidence tiers: the High core
stays 1.0/1.0/0.6, the Med positional trio gets 0.8, and the proxy-heavy
Low tier (challenge-timing 0.4, shot-selection 0.3) trails. Identical
harness, gate, and join (same 3,548 rank-joined players; the severity block
and the three v1 dimension correlations reproduce the v1.1 run exactly, as
they must — that code is untouched):

```
pacifist value vs tier : ρ = +0.073     (v1.1: +0.069)

per-dimension Spearman(value, rank)      n       v1
  over-extension            -0.083    3548    -0.083
  commitment-discipline     +0.116    3548    +0.116
  boost-economy             +0.122    3548    +0.122
  positioning-fit           +0.004    3548      new
  rotation-soundness        +0.209    3548      new
  shadow-quality            +0.028    3548      new
  challenge-timing          -0.317    3548      new
  shot-selection            +0.055    3327      new

per-bucket        n     mean    p50   within-ρ    v1.1 mean
  bronze          90    46.3   40.0    +0.081        44.6
  silver          218   52.2   40.0    -0.047        49.7
  gold            416   52.2   40.0    +0.058        49.8
  platinum        444   54.7   40.0    +0.082        52.1
  diamond         680   56.5   65.7    +0.077        53.9
  champion        847   55.7   62.8    +0.003        53.1
  grand-champion  853   55.2   61.7    +0.030        52.0
```

## Reading the v2 result

1. **Rotation soundness is the best-behaved dimension in the instrument**
   (+0.209, nearly double the previous best). Whether a player recovers
   goalside after their challenge ends is the one place the per-opportunity
   framing found a discipline behavior that climbs the whole ladder — it is
   also the rank series' abandonment/recovery story measured from the
   Pacifist side.

2. **Challenge timing is strongly inverted (−0.317), and the sign is
   diagnostic, not broken.** The time-to-ball proxy penalises a slow
   approach into a ball the opponent reaches first — but deliberately
   slowing the approach to contain, fake, or hold a challenge is *elite*
   technique, so the penalty rate rises with rank. The magnitude (largest
   of any dimension) says the proxy measures something real about pace;
   the sign says it isn't discipline. The design doc rated this dimension
   Low–Med confidence and the 0.4 weight contains the damage; fixing the
   polarity likely needs the outcome of the race (did the opponent actually
   take the ball?) rather than entry-time kinematics alone.

3. **Positioning fit and shadow quality are flat** (+0.004, +0.028) — both
   reduce to a goalside boolean, and goalside/behind-ball share was already
   established as a ladder invariant in the v0 diagnosis. Moving to
   per-opportunity framing did not rescue a condition that everyone
   satisfies at the same rate; these two need *line quality* (depth,
   back-post vs ball-side) rather than a goalside test to discriminate.

4. **The headline holds its shape.** +0.073 overall (CT's inversion at
   weight 0.4 roughly cancels RS's gain at 0.8), the inverted U persists
   (peak Diamond 56.5, GC 55.2), the capped medians stay pinned at 40.0
   through Platinum, and the uncapped Diamond+ medians rise ~7–11 points as
   the new dimensions add mostly-high values for disciplined mid-ladder
   play.

## What this run re-orders

The rubric is now complete; further headline gains are calibration, not
coverage. In priority order: fix challenge-timing's polarity with
race-outcome data (it has the most raw signal to reclaim), give positioning
fit and shadow quality a line-quality measure, then the standing items —
threshold calibration against labeled-adherence replays (rank is still not
the target), and surfacing the score in the app.

---

# v2.1 (`pcfg-v2.1`): challenge timing re-grounded in race outcomes

The one change this version makes: challenge timing no longer judges
entry-time kinematics. The canonical touch stream is bridged into the
context (`bridge::touches` → `MatchContext::with_touches`), and the
dimension now judges **what actually happened**:

- An opportunity is a **realized challenge** — an engagement against a
  non-owned ball in which the player *touches* it. Shadowing and
  containment (closing in without contact) are no longer judged at all,
  which is precisely the elite behavior the kinematic test was
  mis-penalising.
- A challenge is penalised as a **second-strike commit** when an opponent
  won the first touch after the player's commit by more than an even-50/50
  margin (0.25 s).

Identical harness; every other dimension and the severity block reproduce
v2 exactly:

```
pacifist value vs tier : ρ = +0.085     (v2: +0.073, v1.1: +0.069)

  challenge-timing          -0.015    3548     (v2 kinematic: -0.317)
```

The inversion is gone — a 0.30 swing in ρ from one definitional change —
and the headline gains what the inverted drag was costing. The residual is
essentially zero, and that flatness is *structural*, not a failure: in a
rank-matched lobby, realized challenges are contests between equals, so
first-strike win rates hover near symmetric at every bracket — an
outcome-symmetric metric cannot carry rank across the ladder. What it
*can* carry is the per-player, per-moment evidence ("second-strike commit
at 3:41") that the coaching read wants, at an honest weight (0.4, the
design doc's Low–Med tier).

Bucket means shift up ~1 point across the board (Bronze 46.4 → GC 56.3,
Diamond peak 57.6) with the same inverted-U shape and cap dynamics.

Remaining from the v2 list: line-quality measures for positioning fit and
shadow quality, threshold calibration against labeled-adherence replays,
and surfacing the score in the app.

---

# v2.2: shadow quality gets a line-quality fault (guide-grounded, not calibrated)

The v2 diagnosis for shadow quality's flatness (+0.028) was that a goalside
boolean restates a ladder invariant. The fix doesn't need labeled-replay
calibration — the criteria doc's mapping table already carries a
guide-stated number for exactly this gap: O2-1/O2-2 ("string theory")
specify that the 2nd man should trail the engaging teammate by a couple of
pad-lengths — close enough to step in immediately, far enough not to
double-commit. Shadow quality now penalises a **second, independent**
failure mode alongside the goalside test: mean distance to the engaging
teammate exceeding `max_trail_uu` (4000uu, a documented approximation of
the guide's pad-length language, not a literal unit conversion — the same
status as the engagement radii). A patient-but-detached 2nd man now fails
on this condition even when goalside.

Identical harness; every other dimension reproduces v2.1 exactly:

```
pacifist value vs tier : ρ = +0.103     (v2.1: +0.085 — best yet)

  shadow-quality            +0.175    3548     (v2.1: +0.028)

per-bucket        n     mean    p50   within-ρ    v2.1 mean
  bronze          90    45.9   40.0    +0.083        46.4
  silver          218   51.6   40.0    -0.051        52.7
  gold            416   51.9   40.0    +0.063        52.8
  platinum        444   54.5   40.0    +0.091        55.6
  diamond         680   56.4   65.9    +0.078        57.6
  champion        847   55.8   62.0    +0.016        56.8
  grand-champion  853   55.6   62.5    +0.034        56.3
```

Shadow quality moves from the flattest dimension to the second-strongest
(+0.175, behind only rotation-soundness's +0.209) — a 6× gain from one
guide-grounded condition. Holding line *distance*, not just line *side*,
turns out to carry real rank signal: the goalside boolean was catching
players parked on the wrong side of the ball, but missing the more common
mid-ladder failure of a 2nd man who is technically goalside yet too
detached to react. Headline rises to +0.103, the inverted-U shape and cap
dynamics unchanged.

Positioning fit is still flat (+0.004) and still needs its own
line-quality measure; unlike shadow quality's O2-1/O2-2, the design doc's
mapping table gives positioning fit's analogous guide numbers (D2-25's
"~2 car lengths off centre" hybrid-shadow offset) in terms of field
landmarks (centre line, no-man's-land edge, goal line) that
`FieldGeometry` does not yet model beyond `goal_y` — a larger, riskier
lift than shadow quality's direct teammate-to-teammate distance, left for
a follow-up. Threshold calibration against labeled-adherence replays
remains the standing item everything else defers to.

---

# v2.3: positioning fit gets depth/lateral faults — the strongest dimension yet

The feared blocker turned out not to block much. Positioning fit's
guide-stated line quality (D2-24: "do not sit locked in net — use a hybrid
shadow"; D2-25/D2-26: hold "the edge of no-man's-land," "stay mostly
central," biased only "around two car lengths" toward the ball side) needs
field landmarks — but only `own_goal_y`, which `FieldGeometry` already
provides. No new field geometry was needed after all: the two new
thresholds (`min_depth_from_goal_uu`, `max_lateral_offset_uu`) are
extractor-local config, the same status as shadow quality's
`max_trail_uu` — documented approximations, not calibrated.

The dimension now checks, in order, at most one fault per opponent-commit
opportunity: **not goalside** (unchanged from v2), then **parked in net**
(within 700uu of the own goal line instead of holding a forward shadow
depth), then **too wide off centre** (lateral offset beyond 2500uu instead
of staying centrally biased).

Identical harness; every other dimension reproduces v2.2 exactly:

```
pacifist value vs tier : ρ = +0.127     (v2.2: +0.103 — best yet)

  positioning-fit           +0.261    3548     (v2.2: +0.004)

per-bucket        n     mean    p50   within-ρ    v2.2 mean
  bronze          90    44.7   40.0    +0.098        45.9
  silver          218   49.4   40.0    -0.051        51.6
  gold            416   49.9   40.0    +0.071        51.9
  platinum        444   52.2   40.0    +0.096        54.5
  diamond         680   54.0   61.0    +0.088        56.4
  champion        847   53.6   58.5    +0.029        55.8
  grand-champion  853   53.5   57.0    +0.038        55.6
```

Positioning fit moves from the flattest dimension in the instrument to the
**strongest** — clear of rotation-soundness's +0.209 and shadow quality's
+0.175. Two guide-grounded line-quality upgrades in a row have now each
outperformed every threshold that shipped with the original v2 rubric,
which says more about the diagnosis than about either fix being
special: a pure goalside/goalside-adjacent boolean was always going to
restate the v0-established ladder invariant, and *any* independent
distance-based fault condition was likely to add fresh signal once the
possession and engagement primitives were already right. That predicts
where to look next if more gains are wanted — any remaining dimension
still reducible to a single positional boolean is a candidate.

Headline reaches +0.127 (from +0.010 at v0 — an order of magnitude), the
inverted-U shape holds (peak Diamond, dip at Champion/GC), and the capped
medians stay pinned at 40.0 through Platinum. Remaining work is unchanged:
threshold calibration against labeled-adherence replays is still the one
item everything else defers to, now with two more guide-approximated
constants (`min_depth_from_goal_uu`, `max_lateral_offset_uu`) added to the
list of thresholds calibration would refine.
