# Pacifist score — first corpus validation (v0: honest null result)

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
