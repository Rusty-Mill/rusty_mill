# Rank assessment — Grand Champion (incl. SSL)

Seventh and final entry in the per-rank series
([Bronze](rank-assessment-bronze.md) · [Silver](rank-assessment-silver.md) ·
[Gold](rank-assessment-gold.md) · [Platinum](rank-assessment-platinum.md) ·
[Diamond](rank-assessment-diamond.md) ·
[Champion](rank-assessment-champion.md)): what the top of the ladder looks
like in the replay data, what changed since Champion, and — since this
completes the series — what the whole seven-bracket arc says. The bucket
spans four tiers (GC 1–3 + Supersonic Legend, which the corpus absorbs into
`grand-champion`).

## Data note

**250 manifest replays; 248 retrievable** — two return HTTP 404 from
ballchasing (deleted upstream since the manifest was built; noted, not
fixable). 22 lobbies carried a borderline out-of-band player (tier-17/18
Champions); the 16 that reached the clean stage were dropped at the player
level. Division balance among clean observations: 257 GC1 / 308 GC2 /
219 GC3 / **42 SSL** — read SSL splits as indicative only.

**Coverage gate:** 9 of 248 (**3.6%**) — completing the series' cleanest
sociological curve: **match abandonment falls monotonically from 23% at
Bronze/Silver to 3.6% at GC**, a ~6× drop across the ladder. Of the 9:
3 involve leavers (one a 98-second aborted lobby where everyone left, one a
leave-and-rejoin), 5 are pure AFK-while-connected (longest 85 s), and one
46-second fragment was caught by the *older* min-frames confidence check
(too short to analyze at all) — every exclusion verified individually.

**Every number below uses the 238 clean replays — 826 confident, in-bracket
player-observations**, scored through the shipped, unchanged
`fitted_config.json`. Within-bracket correlations from an isolated scratch
manifest; no committed artifact refit.

## Headline: the summit, measured

| | GC/SSL | Champion | Diamond | Platinum | Gold | Silver | Bronze |
|---|---|---|---|---|---|---|---|
| Composite | **73.4** / 74.9 | 64.2 | 55.8 | 43.7 | 35.7 | 30.1 | 21.7 |
| Quartiles | p25 67.2 · p75 80.2 | 57.2 · 71.9 | 47.9 · 63.4 | 36.0 · 51.0 | 28.1 · 42.0 | 21.5 · 36.3 | 13.9 · 26.4 |
| First-man | 63.9 | 60.0 | 56.6 | 50.9 | 45.7 | 37.5 | 28.9 |
| **Second-man** | **57.7 ↓** | 60.7 | 59.9 | 56.8 | 47.9 | 46.1 | 36.9 |
| **General fundamentals** | **79.7** | 65.6 | 54.2 | 38.5 | 30.6 | 23.8 | 15.8 |
| Modal licence | **Pacifist Master (69%)** | PM (33%) | Plat+Dia | Gold+Plat | Gold | Silver/Unr. | Unranked |

- Composite +9.2 — the **predicted ceiling compression did not
  materialize** (Champion's write-up expected < +8; honesty requires saying
  so). The absolute scale still had headroom; what *did* saturate is the
  licence banding (69% in the top band). Composite stays monotone through
  all four tiers including SSL (70.5 → 74.0 → 75.2 → **77.8**) — the rubric
  tracks rank to the very top.
- **Second-man discipline *falls* at GC** (60.7 → 57.7) while first-man
  rises and fundamentals soar — the first sub-score regression of the
  series, and it's diagnostic, not noise (below).

The final seven-bracket ladder table, sorted by GC position — read it
bottom-up and the series' whole story is visible in one screen:

| metric | weight | Bron | Silv | Gold | Plat | Diam | Cham | **GC** |
|---|--:|--:|--:|--:|--:|--:|--:|--:|
| transition_readiness | 0.000 | 32.7 | 41.3 | 45.1 | 52.2 | 50.4 | 51.4 | **47.9** |
| challenge_timing | 0.000 | 36.3 | 41.6 | 49.1 | 52.2 | 52.7 | 52.5 | **48.9** |
| central_support_fraction | 0.030 | 41.3 | 43.7 | 42.4 | 51.3 | 51.5 | 51.1 | 50.7 |
| first_touch_value | 0.002 | 28.4 | 40.8 | 50.3 | 50.0 | 53.8 | 53.0 | 52.0 |
| recovery_speed | 0.005 | 45.5 | 51.1 | 50.5 | 48.3 | 51.6 | 53.1 | 54.3 |
| **double_commit_rate** | 0.019 | 4.9 | 31.0 | 38.7 | 57.7 | 67.9 | 69.7 | **57.9 ↓** |
| **possession_retention** | 0.000 | 58.1 | 52.2 | 48.0 | 44.8 | 48.1 | 50.4 | **59.1 ↑** |
| goalside_discipline_1st | 0.000 | 19.4 | 32.7 | 43.0 | 46.9 | 52.0 | 57.3 | 62.3 |
| overcommit_rate | 0.007 | 3.4 | 28.6 | 41.6 | 50.6 | 57.2 | 62.3 | 68.3 |
| ball_chase_index | 0.002 | −1.7 | 25.3 | 34.7 | 43.6 | 59.4 | 63.1 | 69.3 |
| facing_ball_share | 0.017 | −6.5 | 20.0 | 32.5 | 44.5 | 63.3 | 67.3 | 70.6 |
| boost_starvation | 0.002 | −4.2 | 23.6 | 41.1 | 53.1 | 61.6 | 64.4 | 70.7 |
| goalside_discipline_team | 0.000 | 14.0 | 30.2 | 43.3 | 48.8 | 57.9 | 62.7 | 72.1 |
| pace | 0.012 | −5.6 | 16.8 | 29.9 | 39.9 | 58.7 | 70.6 | 83.9 |
| **aerial_presence** | 0.042 | 16.2 | 18.7 | 25.6 | 33.2 | 51.4 | 64.4 | **86.3** |
| **reverse_driving** | 0.036 | 7.4 | 20.0 | 26.7 | 41.6 | 58.3 | 71.8 | **88.6** |
| **boost_management** | 0.054 | −3.8 | 11.8 | 26.3 | 33.5 | 51.2 | 67.1 | **90.0** |
| support_spacing (band, uu) | 0.008 | 2674 | 2970 | 2984 | 3112 | 3193 | 3173 | **3017** |

Three storylines resolve in this table:

1. **The chronic laggards finish on top.** Boost management — *below the
   ladder's floor* at Bronze, the #1 leak for seven straight brackets —
   ends at **90%**. Aerial presence at **86%**. Reverse driving at **89%**.
   The metrics that defined the bottom of every earlier table are the
   summit's best; the series' whole shape is these three lines crossing
   everything else.
2. **The craft cluster never had its bracket.** Transition readiness,
   challenge timing, central support, first-touch value, recovery speed —
   the entire bottom of the GC table sits at 48–54%, and two of them are
   *lower* at GC than at Platinum. Across seven brackets, challenge timing
   moved 13 points total while pace moved 90. Either the ladder genuinely
   never demands these (unlikely), or — more probably — these
   textbook-shaped metrics stop *describing* good play at the top (see the
   inversions, next).
3. **Two U-turns, both meaningful.** `possession_retention` completes a
   full U (58 → 45 at Platinum → **59**): low ranks retain by accident
   (slow, close play), the mid-ladder trades touch quality for tempo, and
   the top *reclaims retention at speed* — GC's biggest single-bracket
   riser (+8.7). And `double_commit_rate` **regresses** 12 points on the
   ladder scale while *inverting within the bracket* (+0.158, echoing
   Silver): higher GCs double-commit **more**. At SSL pace, two cars on the
   ball is coordinated pressure with a planned recovery, not indiscipline —
   the mid-ladder's cardinal sin becomes a top-ladder weapon, and the
   falling second-man sub-score (57.7) plus the sharp spacing contraction
   (3173 → **3017 uu**, below the fitted band's center, with tighter still
   correlating higher within-bracket, −0.106) all say the same thing: **GC
   abandons the textbook two-man shape the mid-ladder rubric was fitted
   on.** This is the honest boundary of the rubric — at the top, its role
   template reads deliberate style as leak.

## The leaks: full diversification

`boost_management` 273 (33%), `aerial_presence` 213 (26%),
`central_support_fraction` **162 (20%)**, `reverse_driving` 80 (10%),
`facing_ball_share` 78 (9%) — the duopoly that owned 94% of Platinum's
leaks is down to 59%, with the support-shape metric surging to a real third
place (partly genuine, partly the role-template boundary above). The
coaching story at GC is individual, not bracket-wide.

Within-bracket, the two old names still crown the divisions — with the
**strongest signals of the entire series**: `aerial_presence` **+0.223**
and `boost_management` **+0.172** separate GC1 from SSL more sharply than
they separated any lower bracket's divisions. From Bronze to SSL, one
sentence survives every bracket: *boost and air decide who climbs.*

## What changed on the field, Champion → GC

**Boost:** the lean-tank endpoint — collection 414 bpm, spend 402 bcpm
(each ~+7%), average gauge **47.4, the lowest since Bronze** (the seven-bracket
gauge: 46 → 51 → 51 → 52 → 51 → 50 → **47**), full-boost time at a series
low (12.0%) and zero-boost time *rising* (14.5%). The ladder's boost lesson,
end to end: flow, deny (steals 471, +6%), and run lean — never hoard.

**Movement:** speed 1429 (+41); supersonic 16.3% (+28%); ground time under
60% for the first time (59.8%); high-air 8.7% (+28%); powerslides 58.6/match
(8× Bronze). GC spends 40% of the match airborne.

**Positioning:** everything contracts toward the ball — distance-to-ball
2417 uu (series low, −112), teammate spacing 2976 (−141, the sharpest move
of the series), behind-ball share *up* to 74.7%. Field thirds barely move
(defensive 46.8%); last-defender concessions tick up (1.36 → 1.43) — at GC
pace, more shots simply reach the last man. The compact, ball-near,
behind-ball GC shape is the series' final positioning statement: **the top
plays closer, not wider.**

**Demos: fully weaponized.** 1.19/match (+28%, the biggest step yet), 55%
of players — from a 0.6 flatline through four brackets to a routine
pressure tool.

## Mechanics: the air-control era completes; first-contact shooting peaks

| skill | B | S | G | P | D | C | GC | rate (GC) |
|---|--:|--:|--:|--:|--:|--:|--:|--:|
| aerial | 67% | 72% | 93% | 97% | 100% | 100% | 100% | **12.88** |
| **air_dribble** | 6% | 9% | 11% | 17% | 29% | 47% | **72%** | 1.47 |
| ceiling_play | 33% | 36% | 45% | 41% | 50% | 59% | **73%** | 1.39 |
| wall_play | 56% | 56% | 69% | 74% | 80% | 86% | **92%** | 2.44 |
| double_touch | 13% | 8% | 10% | 12% | 15% | 20% | **31%** | 0.45 |
| ground_dribble | 11% | 13% | 18% | 24% | 25% | 31% | 37% | 0.47 |
| demo | 31% | 37% | 36% | 41% | 43% | 48% | **55%** | 1.19 |
| power_shot | 68% | 85% | 92% | 92% | 95% | 97% | 96% | 4.26 |
| **redirect** | 63% | 85% | 93% | 96% | 95% | 97% | 97% | **4.17 ↓** |
| flick | 53% | 59% | 57% | 51% | 45% | 40% | **37%** | 0.47 |

- **Air dribbling goes from 6% of Bronze players to 72% of GCs** — a 12×
  presence climb and the single most rank-diagnostic mechanic in the
  catalog. With ceiling play at 73% and double-touches at 31%, multi-touch
  air control is the top's defining mechanical vocabulary.
- **The redirect rate declines for the first time** (4.47 → 4.17/match)
  while possession retention jumps — the top shoots *less* on first
  contact and keeps more. Six brackets built the first-touch shot; GC
  starts declining it.
- The **flick's seven-bracket slide bottoms out at 37%** (Silver peak 59%)
  — the definitive answer on that mechanic: the ladder progressively
  abandons the pop-flick in favor of carried air plays.
- Kickoff first-touch presence is the series' one true constant (74–81%
  across all seven brackets) — kickoffs are contested the same everywhere;
  what changes is everything after.

## The limits of Grand Champion, synthesized

1. **The rubric itself becomes the measurement boundary.** Second-man
   score falling, double-commits "regressing", spacing "too tight",
   transition/challenge metrics flat-to-down — the coherent reading is that
   GC plays a different game than the textbook the mid-ladder rubric
   rewards, not a worse one. Assessing *within* the top bracket now needs
   the rank-relative layer (`scoring::relative`, which this corpus
   expansion feeds) and the value model more than the absolute composite.
2. **Air and boost still aren't finished** — the series' most durable
   fact. Even at 86–90% of the calibrated ladder, they post the strongest
   within-bracket division signals of all seven write-ups. The gap between
   GC1 and SSL is *still* mostly the air game.
3. **The lean tank has a cost curve.** Zero-boost time rises at the top;
   starvation is a top-3 within-bracket signal (−0.128). SSL runs the same
   lean economy with fewer stalls — the last boost skill is surviving
   empty.
4. **Craft metrics are unconquered or unmeasurable** — first-touch value,
   challenge timing, transition readiness never rose past ~54% for anyone.
   Whether that's headroom above SSL or metric mis-specification at the
   top, it marks where this rubric stops explaining rank and the value
   model (ΔV) has to take over.

## The seven-bracket arc, in five sentences

1. **Bronze → Silver** corrects the pathologies (ball-watching, chasing,
   double-commits) that sit below the entire ladder's floor.
2. **Silver → Gold → Platinum** is the discipline era: speed arrives, then
   the restraint to channel it (the double-commit lesson), while boost and
   air stay pinned at the bottom of the table.
3. **Diamond** breaks the bottleneck — boost and air cross the ladder's
   midpoint in one bracket, the largest composite jump of the series.
4. **Champion** completes the fundamentals inversion (general fundamentals
   overtakes role play) and diversifies the leak profile.
5. **GC/SSL** reclaims possession at speed, compresses the pair around the
   ball, weaponizes demos and coordinated double-pressure — and the two
   metrics that were broken at Bronze finish as the summit's best, still
   deciding who reaches the very top.

Constant across all seven: the ~50 average boost gauge (flow economy, never
storage), kickoff contention, and the primacy of boost + air as the
ladder's currency.

## Reproducing / extending this analysis

```sh
BC_TOKEN=<token> python assets/corpus/refresh_corpus_replays.py --bucket grand-champion
python assets/corpus/refresh_manifest_playlist.py --skip-missing

./target/release/replay-scoring --config assets/corpus/fitted_config.json --json out.json <replay>
./target/release/replay-analyzer --bc-stats out.json <replay>
./target/release/replay-skills --json out.json <replay>
# filter to "confidence": "ok" and manifest tier 19..22
```

As with all prior brackets, `calibrate` was **not** run against the full
manifest; within-GC numbers come from an isolated scratch manifest, and the
committed `fitted_config.json`/`value_model.json` are byte-identical to
before. With this entry, **every corpus bucket's replays are now present
locally** (1,055 of 1,057 files; 2 deleted upstream) — a full-corpus
recalibration (`scripts/calibrate_scoring.sh`) is now possible if the
fitted artifacts should absorb the new Bronze/Silver data, but that is
deliberately left as a separate, explicit decision.
