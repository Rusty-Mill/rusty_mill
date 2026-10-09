# Rank assessment — Silver

Second entry in the per-rank series (after
[`rank-assessment-bronze.md`](rank-assessment-bronze.md)): what Silver play
looks like in the replay data, where it sits against the full ranked ladder,
what actually changed since Bronze — measured with identical tooling on both
brackets — and what closes the gap to Gold.

## Data note: growing the thinnest bucket

Silver was the corpus's thinnest bucket (48 replays). Topping it up surfaced
two real data-tooling problems, both fixed in `expand_manifest.py`:

1. **ballchasing's pagination drops the `max-rank` filter.** The search
   response's `next` URL comes back with `max-rank=` empty while `min-rank`
   survives (observed live), so page 2+ of a silver-band search silently
   became "Silver I *and above*" — champion/GC lobbies. The Bronze pull never
   tripped this because it filled from page 1. `search_ids` now rebuilds the
   URL from its own params plus the `after` cursor instead of trusting `next`.
2. **Requiring all four players ranked rejects nearly every modern low-rank
   upload.** On current Silver uploads ballchasing usually has a rank for
   *only the uploader* (sampled page-1 candidates: mostly 1 of 4 ranked), so
   the old all-four rule produced **zero** acceptances — and is probably why
   Silver was the thinnest bucket in the first place. `entry_from_detail` now
   admits a replay when ballchasing's own **lobby rank band**
   (`min_rank`..`max_rank`, which bounds even the players whose individual
   ranks are hidden) sits inside a single bucket, at least `--min-ranked`
   (default 2) players carry a tier, and every *known* tier is in-band (a
   known out-of-band tier — i.e. a visible smurf — rejects the replay).
   `tiers`/`ranks` then cover the known players only, and the calibrate
   rank-join simply skips unranked lobby-mates.

With both fixes: +32 new entries (all known tiers strictly 4–6), 80 total,
all `.replay` files downloaded. One new entry was then dropped again — its
header carries no playlist name and, with only 3 of 4 players ranked, the
header-side `inferred-ranks` verification can't fire, so it can't be
independently confirmed as ranked-doubles; rather than weaken that
defense-in-depth, it's out. **Final: 79 Silver replays** — 48 legacy (older
uploads, all four players ranked) + 31 new (current, 2–3 ranked players
each).

Two sample-composition caveats, then the gate:

- **Mixed upload eras.** The legacy 48 are older-season uploads; the new 31
  are current. Nothing here suggested a systematic era split, but "Silver"
  below is a blend, not a single patch snapshot.
- **Rank-join coverage.** 282 of 316 player-slots (79×4) rank-joined; the
  unjoinable ones are lobby-mates whose ranks ballchasing hides. Joined
  players on the new entries skew toward uploaders (people who bother with
  replay tooling), plausibly the more improvement-minded end of Silver.

**Coverage gate (from the Bronze work, now on from the start):** 18 of 79
replays (23%) failed the lobby-completeness gate — *identical* to Bronze's
7/30 (23%), which reads as a systemic low-rank base rate, not a Bronze quirk.
Verifying every one: 16 involve a mid-match leaver (track coverage 7–86%;
four of those players also idled ≥20s before quitting — the rage-quit
pattern), and 2 are pure AFK-while-connected (a >20s mid-game freeze with no
disconnect). Four of the leaver replays show a fifth track — a player who
disconnected and **rejoined** (two partial tracks for the same name), which
the gate correctly treats as a disrupted lobby. No false positives: every flagged stretch was
re-checked against the other players' simultaneous speeds and kickoff/goal
dead time.

**Every number below uses only the 61 clean replays — 216 player-observations
(40 Silver I / 89 Silver II / 87 Silver III), all `confidence: ok`, all
individually tier 4–6, zero out-of-bracket players.** Scored through the
shipped, unchanged `fitted_config.json`; no calibration artifact was
refit (the within-Silver correlations came from an isolated scratch
manifest, same method as Bronze).

## Headline: one bracket up, still the bottom third of the rubric

| | Silver | Bronze (same tooling) |
|---|---|---|
| Composite (0–100) | **mean 30.1**, median 28.3, σ=12.7 | mean 21.7, median 20.1, σ=12.0 |
| Quartiles | p25 21.5 · p75 36.3 | p25 13.9 · p75 26.4 |
| First-man discipline | 37.5 | 28.9 |
| Second-man discipline | 46.1 | 36.9 |
| **General fundamentals** | **23.8** | 15.8 |
| Licence bands | 35% Unranked, 34% Silver, 20% Gold, 10% Plat+ | 70% Unranked, 21% Silver, 8% higher |

Three things to read out of that:

- **The whole profile shifts up ~8–9 points**, evenly across all three
  sub-scores — Silver is Bronze plus consistency, not a different shape of
  player. General fundamentals stays the weakest third at both brackets.
- **A top-half Bronze player is a bottom-quartile Silver** (Bronze median
  20.1 ≈ Silver p25 21.5). The brackets genuinely overlap; the boundary is
  soft.
- **The rubric's own licence bands now roughly agree with the bracket** (35%
  below Silver Licence vs Bronze's 70%) — external validation that the
  absolute composite tracks the real rank boundary.
- **Composite is monotonic within the bracket** (Silver I 27.6 → II 30.1 →
  III 31.2), unlike Bronze's flat plateau — differentiation between divisions
  has begun.

## The #1 leak: still boost, but the aerial gap has opened

`main_leak = boost_management` for **170 of 216 (79%)** — down from Bronze's
91% but still overwhelming. The interesting movement is second place:
`aerial_presence` is now the main leak for 28 players (13%, vs 4% at Bronze),
and it's also the **second-strongest within-Silver rank signal** (Spearman
+0.195 vs tier). At Bronze the air game was too uniformly absent to separate
anyone; by Silver, who can be an aerial threat has started deciding games.
`reverse_driving` (14) and `central_support_fraction` (3) trail.

## What actually improved from Bronze — and what didn't

Measured metric-by-metric on the shipped config's corpus-wide curves
(`pos` = where the bracket mean falls between the ladder-wide floor and
ceiling; Bronze column from the same pipeline):

| metric | weight | Bronze pos | Silver pos | Δ |
|---|--:|--:|--:|--:|
| facing_ball_share | 0.017 | −6.5% | 20.0% | **+26.5** |
| boost_starvation | 0.002 | −4.2% | 23.6% | **+27.8** |
| ball_chase_index | 0.002 | −1.7% | 25.3% | **+27.0** |
| double_commit_rate | 0.019 | 4.9% | 31.0% | **+26.1** |
| overcommit_rate | 0.007 | 3.4% | 28.6% | **+25.2** |
| pace | 0.012 | −5.6% | 16.8% | +22.4 |
| goalside_discipline_team | 0.000 | 14.0% | 30.2% | +16.2 |
| **boost_management** | **0.054** | −3.8% | **11.8%** | +15.6 |
| goalside_discipline_1st | 0.000 | 19.4% | 32.7% | +13.3 |
| reverse_driving | 0.036 | 7.4% | 20.0% | +12.6 |
| first_touch_value | 0.002 | 28.4% | 40.8% | +12.4 |
| transition_readiness | 0.000 | 32.7% | 41.3% | +8.6 |
| recovery_speed | 0.005 | 45.5% | 51.1% | +5.6 |
| challenge_timing | 0.000 | 36.3% | 41.6% | +5.3 |
| central_support_fraction | 0.030 | 41.3% | 43.7% | +2.4 |
| **aerial_presence** | **0.042** | 16.2% | **18.7%** | **+2.5** |
| possession_retention | 0.000 | 58.1% | 52.2% | −5.9 |

Plus one band metric: `support_spacing` moves from **2674 uu (below the
2824–3305 uu target band — teammates crowding each other) into the band at
2970 uu** — Silver pairs have learned to not stand on top of each other.

The pattern is striking: **the five biggest improvements are exactly Bronze's
five worst metrics** — ball-watching, boost starvation, ball-chasing,
double-commits, overcommits all jump ~25 points of ladder range. What Bronze
→ Silver actually is, in the data, is the partial correction of Bronze's
pathologies. Meanwhile the two *highest-weighted* fundamentals barely move
relative to the ladder: `boost_management` (+15.6 but still only at 11.8%)
and especially `aerial_presence` (+2.5, essentially stagnant at 18.7%) — the
two things that keep Silver in the bottom third and that the leak counter
says now dominate.

The one regression, `possession_retention` (58.1% → 52.2%, unweighted), reads
as a side effect of pace: Silver plays ~6% faster and challenges more, and
first-touch retention dips slightly while everything else speeds up.

**On the field (bcstats deltas, Bronze → Silver):** average gauge 46 → 51;
time-at-zero-boost 19.2% → 15.2%; collection 227 → 263 bpm; average speed
1151 → 1221 uu/s; powerslides 7.1 → 10.7 per match (+50% — the turning
mechanic is being adopted); teammate spacing +279 uu; time facing the ball
48.6% → 45.4% (less ball-watching); goals conceded as last defender 1.72 →
1.46. Time-distribution barely changes (defensive third ~49%, behind-ball
~70–72%) — *where* Silver stands is Bronze-like; *how it moves and manages
resources* is what improved.

## Mechanics: shot quality arrives, ball control still absent

The two standout skill deltas from Bronze are both single-contact **shot
quality** mechanics:

| skill | Bronze %players ≥1 | Silver %players ≥1 | attempts/match (Silver) |
|---|--:|--:|--:|
| **power_shot** | 68% | **85%** | 2.19 (was 1.57) |
| **redirect** | 63% | **85%** | 2.25 (was 1.47) |
| aerial | 67% | 72% | 2.42 |
| flick | 53% | 59% | 0.84 |
| demo | 31% | 37% | 0.62 |
| wall_play | 56% | 56% | 0.99 |
| ceiling_play | 33% | 36% | 0.52 |
| ground_dribble | 11% | 13% | 0.16 |
| air_dribble | 6% | 9% | 0.19 |
| double_touch | 13% | 8% | 0.09 |

Silver hits the ball **hard and on purpose** far more often — power shots and
redirects are now near-universal (85%) and ~50% more frequent per match. But
sustained ball control (ground/air dribble, double touch) stays where Bronze
left it: single digits to low teens, ~0.1–0.2 attempts/match. Silver offense
is Bronze offense executed harder — first-contact shots — not a new kind of
offense.

## Within-Silver: what separates Silver I from Silver III

The isolated within-bracket calibration (284 obs incl. low-confidence, tiers
4–6 only; restricted-range caveats apply) gives a much stronger internal
signal than Bronze's flat plateau. Strongest tier-over-tier climbers, in
order:

1. `boost_management` **+0.257** — boost economy keeps separating players
   *inside* the bracket, on top of separating the bracket from Bronze.
2. `aerial_presence` **+0.195** — the emerging differentiator.
3. `agility` +0.179 / `pace` +0.174 — general movement quality.
4. Less `reverse_driving` (−0.160) and shorter `boost_starvation` runs
   (−0.141), both the good direction.

One genuinely interesting inversion: `double_commit_rate` correlates
**positively** with tier inside Silver (+0.174) — higher Silvers double-commit
*more*, not less. Combined with rising pace and the slight
possession-retention dip, the picture is that **eagerness/speed arrives
before the discipline to channel it** — a Silver III plays faster and
challenges more than a Silver I, and pays for it in double-commits until the
rotation discipline catches up (the corpus-wide direction, lower-is-better,
reasserts itself above this bracket).

## The limits of Silver, synthesized

1. **Boost economy is still the binding constraint.** Main leak for 79% of
   players, strongest within-bracket rank signal, and the bracket mean sits
   at just 11.8% of the ladder's calibrated range despite the +15 jump from
   Bronze. Silver collects more but also *spends* more (bcpm 221 → 250); the
   gauge discipline that would let the speed gains stick isn't there yet.
2. **The air game has stalled while everything else improved.** Aerial
   presence moved just +2.5 ladder-points from Bronze (16.2 → 18.7%) while
   the bracket's other pathologies jumped ~25 — and it's now the #2 leak and
   #2 within-bracket separator. High-air time is still only 3.5% of the
   match. Silver plays the same near-ground game as Bronze, faster.
3. **Speed has outrun structure.** Pace +22 ladder-points, powerslides +50%,
   but double-commits *rise* with tier inside the bracket and possession
   retention dips — the bracket is learning to go fast before learning when
   not to.
4. **Offense is still first-contact-only.** Power shots and redirects are
   near-universal now, but nobody manufactures a second touch: dribbles, air
   dribbles, and double touches remain a rounding error. Scoring still
   depends on the ball arriving in a hittable spot.
5. **Defensive positioning is habit, not reading.** Time-in-defensive-third
   and behind-ball shares are unchanged from Bronze (~49%, ~72%) and
   goals-against-as-last-defender only edges down (1.72 → 1.46) — Silver
   stands in roughly the right places more reliably, but the last-line
   breakdown rate says the *reading* of incoming plays hasn't materially
   improved.

## How to improve beyond Silver

Ordered by weight × distance-below-ladder, same rule the rubric uses to pick
a leak:

1. **Boost economy, still, and now with a specific shape.** Silver's problem
   is no longer Bronze's "runs to zero and sits there" (time-at-zero already
   fell 19% → 15%) — it's *churn*: collection and spend both rose ~15% and
   the gauge still averages barely half. Focus shifts from "don't be empty"
   to **spend less for the same movement**: feather boost instead of holding
   it, take pad routes on the way back to position, arrive at the play with
   ≥50 rather than arriving empty having boosted the whole way.
2. **Start the air game deliberately.** This is the bracket where the aerial
   gap begins deciding rank (leak #2, separator #2) and it's the one metric
   that didn't move from Bronze. Concretely: fast-aerial training packs, and
   in games take every uncontested high ball in your own half — the data
   says Silver already *attempts* aerial touches (72% of players) but at
   Bronze frequency and height (3.5% high-air time).
3. **Re-attach discipline to the new speed.** The double-commit inversion is
   the tell: when you feel yourself going faster than your rotation, that's
   the moment to hold — the corpus-wide direction (fewer double-commits =
   higher rank) reasserts itself immediately above this bracket, so the
   Silver III habit of "faster = challenge more" is exactly what Gold
   punishes.
4. **Convert shot power into shot placement.** Power shots and redirects are
   near-universal at Silver — the differentiation upward is no longer *can
   you hit it hard* but *where*. Aim for corners/far-post off redirects
   rather than "hard at net" (the `first_touch_value` climb within the
   bracket, +12 ladder-points from Bronze, is this signal starting to pay).
5. **Keep the ball-watching trend going.** The single biggest Bronze→Silver
   improvement (+26.5 ladder-points) — the scanning habit is forming; it just
   needs to keep falling toward the 0.33 share the ladder's top anchors at.
6. **Ball control can wait until Gold.** Dribble/air-dribble/double-touch
   rates are unchanged from Bronze and nothing in the within-bracket signal
   says they separate Silver tiers yet — fundamentals 1–3 are worth strictly
   more rank per practice-hour here.

## Reproducing / extending this analysis

```sh
# grow the Silver sample further (pagination + partial-rank acceptance fixed)
BC_TOKEN=<token> python assets/corpus/expand_manifest.py --bucket silver --per-tier 250
BC_TOKEN=<token> python assets/corpus/refresh_corpus_replays.py --bucket silver
python assets/corpus/refresh_manifest_playlist.py --skip-missing

# score through the shipped (unchanged) production rubric; the
# lobby-completeness gate marks incomplete lobbies low-confidence automatically
./target/release/replay-scoring --config assets/corpus/fitted_config.json --json out.json <replay>
./target/release/replay-analyzer --bc-stats out.json <replay>
./target/release/replay-skills --json out.json <replay>
# then filter aggregation to reports where "confidence": "ok",
# and to players whose own manifest tier is 4..6
```

As with Bronze, `calibrate` was **not** run against the full manifest (948 of
the corpus's replay files aren't downloaded locally; a full-manifest run would
refit the committed artifacts off only the local low-rank files). The
within-Silver numbers came from an isolated scratch manifest; the committed
`fitted_config.json`/`value_model.json` are byte-identical to before.
