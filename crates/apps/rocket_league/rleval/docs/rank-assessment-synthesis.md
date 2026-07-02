# How the ladder works — the rank-assessment series, synthesized

The capstone of a seven-part, replay-measured assessment of every Rocket
League ranked-doubles bracket. Each entry examined one bracket in depth —
this document is the cross-bracket layer: the master tables, the arcs that
only appear when all seven are laid side by side, and a per-bracket practice
guide. Detail and methodology live in the per-bracket entries:

[Bronze](rank-assessment-bronze.md) · [Silver](rank-assessment-silver.md) ·
[Gold](rank-assessment-gold.md) · [Platinum](rank-assessment-platinum.md) ·
[Diamond](rank-assessment-diamond.md) · [Champion](rank-assessment-champion.md) ·
[Grand Champion / SSL](rank-assessment-grand-champion.md)

**Method, in one paragraph.** Every bracket was measured with the *same*
pipeline on real ranked-2v2 replays: decode → canonical match →
decision-discipline scoring through the shipped `fitted_config.json`
(unchanged for the entire series, so brackets are directly comparable) →
`bcstats` kinematics → mechanical-skill detection. Every replay passed the
lobby-completeness gate (all four players present and active — leavers/AFKs
drop the whole lobby to low confidence), and every player was individually
rank-verified against ballchasing tiers, with out-of-bracket lobby-mates
excluded person-by-person. Totals: **3,432 confident, in-bracket
player-observations from 952 clean replays** (of 1,055 processed; the
lobby-completeness gate excluded 101 replays, player-level tier filters and
rank-join gaps the rest). All
correlations quoted "within bracket" come from isolated scratch
calibrations that never touched the committed artifacts.

## The master table: composite by bracket

| bracket | n (players) | composite mean | p25 | median | p75 | step |
|---|--:|--:|--:|--:|--:|--:|
| Bronze | 90 | 21.7 | 13.9 | 20.1 | 26.4 | — |
| Silver | 216 | 30.1 | 21.5 | 28.3 | 36.3 | +8.4 |
| Gold | 415 | 35.7 | 28.1 | 35.1 | 42.0 | +5.6 |
| Platinum | 430 | 43.7 | 36.0 | 42.8 | 51.0 | +8.0 |
| Diamond | 660 | 55.8 | 47.9 | 55.6 | 63.4 | **+12.1** |
| Champion | 795 | 64.2 | 57.2 | 65.1 | 71.9 | +8.4 |
| GC/SSL | 826 | 73.4 | 67.2 | 74.9 | 80.2 | +9.2 |

Monotone at every step, monotone *within* every bracket's divisions
(including SSL at 77.8), and each bracket's modal licence band tracked the
real rank all seven times — the absolute rubric's strongest external
validation to date. The steps are not uniform: Silver→Gold is the smallest
climb, Platinum→Diamond by far the largest (it's where the two chronic
bottlenecks break — see below). Adjacent brackets overlap roughly
p25-to-median: a top-quartile player is statistically already playing at
the next bracket's median.

## The master table: every metric's road up the ladder

Position of each bracket's mean on the corpus-calibrated floor→ceiling line
of each weighted metric (100% = the calibrated ladder ceiling; negative =
below the floor of the entire ranked population):

| metric | w | Bron | Silv | Gold | Plat | Diam | Cham | GC |
|---|--:|--:|--:|--:|--:|--:|--:|--:|
| boost_management | 0.054 | **−3.8** | 11.8 | 26.3 | 33.5 | 51.2 | 67.1 | **90.0** |
| aerial_presence | 0.042 | 16.2 | 18.7 | 25.6 | 33.2 | 51.4 | 64.4 | **86.3** |
| reverse_driving | 0.036 | 7.4 | 20.0 | 26.7 | 41.6 | 58.3 | 71.8 | **88.6** |
| pace | 0.012 | **−5.6** | 16.8 | 29.9 | 39.9 | 58.7 | 70.6 | 83.9 |
| facing_ball_share | 0.017 | **−6.5** | 20.0 | 32.5 | 44.5 | 63.3 | 67.3 | 70.6 |
| boost_starvation | 0.002 | **−4.2** | 23.6 | 41.1 | 53.1 | 61.6 | 64.4 | 70.7 |
| ball_chase_index | 0.002 | **−1.7** | 25.3 | 34.7 | 43.6 | 59.4 | 63.1 | 69.3 |
| overcommit_rate | 0.007 | 3.4 | 28.6 | 41.6 | 50.6 | 57.2 | 62.3 | 68.3 |
| double_commit_rate | 0.019 | 4.9 | 31.0 | 38.7 | 57.7 | 67.9 | 69.7 | 57.9 ↓ |
| goalside_discipline_team | — | 14.0 | 30.2 | 43.3 | 48.8 | 57.9 | 62.7 | 72.1 |
| goalside_discipline_1st | — | 19.4 | 32.7 | 43.0 | 46.9 | 52.0 | 57.3 | 62.3 |
| possession_retention | — | 58.1 | 52.2 | 48.0 | 44.8 | 48.1 | 50.4 | 59.1 ⌣ |
| first_touch_value | 0.002 | 28.4 | 40.8 | 50.3 | 50.0 | 53.8 | 53.0 | 52.0 |
| central_support_fraction | 0.030 | 41.3 | 43.7 | 42.4 | 51.3 | 51.5 | 51.1 | 50.7 |
| challenge_timing | — | 36.3 | 41.6 | 49.1 | 52.2 | 52.7 | 52.5 | 48.9 |
| transition_readiness | — | 32.7 | 41.3 | 45.1 | 52.2 | 50.4 | 51.4 | 47.9 |
| recovery_speed | 0.005 | 45.5 | 51.1 | 50.5 | 48.3 | 51.6 | 53.1 | 54.3 |
| support_spacing (uu) | 0.008 | 2674 | 2970 | 2984 | 3112 | 3193 | 3173 | 3017 |

Read by rows, three families:

- **The 90-point climbers** (top of the table): boost management, aerial
  presence, reverse driving, pace — from at-or-below the ladder's floor to
  84–90%. These *are* the ladder, in this rubric's terms.
- **The mid-arc disciplines**: chasing, overcommits, double-commits,
  goalside shape — steep through the mid-ladder, plateauing (or inverting)
  at the top once absorbed.
- **The flat craft cluster** (bottom): first-touch value, challenge timing,
  transition readiness, central support — ~10–15 points of total movement
  in seven brackets. Partly genuine headroom above SSL, partly (per the GC
  entry) textbook-shaped metrics that stop describing elite play.

## The arc in five acts

1. **Bronze → Silver: correcting pathologies.** Bronze sits *below the
   entire ladder's calibrated floor* on five metrics at once
   (ball-watching, boost starvation, chasing, pace, boost economy); the
   Silver step is precisely the correction of those five (~+25
   ladder-points each). What Bronze→Silver *is*, in data, is the partial
   repair of Bronze's worst habits — not new skills.
2. **Silver → Gold → Platinum: the discipline era.** Speed arrives before
   restraint (double-commits *rise* with tier inside Silver, +0.174), then
   restraint catches up (the same metric is Gold's strongest separator at
   −0.185, and Platinum's *best* ladder stat). Meanwhile the two
   highest-weighted fundamentals — boost and air — stay pinned to the
   bottom of the table for four straight brackets, and are 88–94% of all
   main-leaks.
3. **Diamond: the bottleneck breaks.** Boost management and aerial
   presence cross the ladder's midpoint *in one bracket* (+18 points each)
   — the series' largest composite jump (+12.1) — yet remain the top two
   within-bracket separators: the Diamond climb is the air-and-boost climb.
   Multi-touch air control arrives (air dribbling 17% → 29% of players).
4. **Champion: the fundamentals inversion.** General fundamentals
   *overtakes* the role sub-scores for the first time (65.6 vs ~60); the
   leak profile finally diversifies (boost under half); demos wake from a
   five-bracket flatline. The frontier moves from moving the car to
   working the ball.
5. **GC/SSL: the textbook ends.** Possession is reclaimed at speed (the
   retention U-curve closes at 59%, its series high); pairs compress hard
   around the ball (spacing 3173 → 3017 uu, tighter = higher
   within-bracket); coordinated double-pressure re-inverts the
   double-commit metric; and the two Bronze-broken metrics finish as the
   summit's best — still posting the strongest divisional signals of the
   entire series (aerial +0.223, boost +0.172). Boost and air decide who
   climbs, Bronze through SSL.

## The invariants (what never changed)

- **The ~50 boost gauge.** Seven brackets: 46 → 51 → 51 → 52 → 51 → 50 →
  47 — while per-minute flow rose 82% (bpm 227 → 414) and stealing tripled
  in volume. Nobody on the ladder hoards boost; rank buys *throughput and
  denial*, never storage. (The lean tank actually *leans further* at the
  top: GC posts series-low full-boost time and rising zero-boost time.)
- **Kickoff contention.** First-touch presence 74–81% across all seven
  brackets, rate ~1.3–1.6/match — the one skill the whole ladder performs
  identically. Games are decided after the kickoff, not at it.
- **The behind-ball shape.** ~70–75% behind-ball and ~47–50%
  defensive-third at every bracket. *Where* players stand barely moves in
  seven brackets; what changes is speed, resources, and what they do when
  the shape is tested.
- **Demos as a wash** — until Champion (0.6–0.76/match flatline, then
  +22%, +28% steps to 1.19 at GC). The one "invariant" that eventually
  breaks, late.

## The U-curves and reversals (the series' best plot twists)

- **Possession retention** falls four straight brackets
  (58 → 45 at Platinum) then recovers to a series-high 59 at GC: low
  ranks retain by accident (slow, close play), the mid-ladder trades touch
  quality for tempo, the top reclaims retention *at speed*. The
  within-bracket signal flips positive exactly at Platinum — one bracket
  before the bracket-level number turns.
- **Double-commits** are the discipline story twice over: inverted
  (more = higher) inside Silver, the strongest correct-direction separator
  inside Gold, a best-stat by Platinum — then inverted *again* inside GC
  (+0.158), where two cars on the ball is coordinated pressure. The same
  behavior is a leak at Silver speed and a weapon at SSL speed.
- **Support spacing is dialectic:** too tight (Bronze, 2674 uu, below
  band) → progressively wider to a Diamond peak (3193, past band center)
  → recompressing at the top (GC 3017), with *tighter* correlating higher
  within both Champion and GC. Escape your teammate; then learn to come
  back.
- **The flick dies on the way up:** 53% of Bronze players → 59% Silver
  peak → monotone decline to 37% at GC, while air dribbling climbs 6% →
  72% (a 12× presence climb, the most rank-diagnostic mechanic in the
  catalog). The ladder abandons the pop-flick for carried air plays.
- **The redirect peaks at Champion** (4.47/match) and *declines* at GC
  (4.17) while retention jumps — six brackets build the first-contact
  shot; the top starts declining it to keep the ball.

## Mechanics: presence across the ladder (% of players with ≥1/match)

| skill | Bron | Silv | Gold | Plat | Diam | Cham | GC |
|---|--:|--:|--:|--:|--:|--:|--:|
| aerial (touch) | 67 | 72 | 93 | 97 | 100 | 100 | 100 |
| power_shot | 68 | 85 | 92 | 92 | 95 | 97 | 96 |
| redirect | 63 | 85 | 93 | 96 | 95 | 97 | 97 |
| wall_play | 56 | 56 | 69 | 74 | 80 | 86 | 92 |
| boost_steal | 74 | 75 | 84 | 87 | 90 | 93 | 92 |
| ceiling_play | 33 | 36 | 45 | 41 | 50 | 59 | 73 |
| **air_dribble** | **6** | 9 | 11 | 17 | 29 | 47 | **72** |
| ground_dribble | 11 | 13 | 18 | 24 | 25 | 31 | 37 |
| double_touch | 13 | 8 | 10 | 12 | 15 | 20 | 31 |
| demo | 31 | 37 | 36 | 41 | 43 | 48 | 55 |
| **flick** | 53 | **59** | 57 | 51 | 45 | 40 | **37** |
| kickoff_first_touch | 78 | 74 | 76 | 76 | 79 | 78 | 81 |

The waves, in order of arrival: **shot quality** (power shots/redirects,
Silver–Gold) → **aerial volume** (Gold–Diamond) → **multi-touch air
control** (air dribble/ceiling, Diamond–GC) → **ground carry** (still
arriving at GC; 37%). Sustained ball control is the last mechanical
frontier at every bracket that has it ahead of them.

## The abandonment curve (a data-quality by-product)

The lobby-completeness gate, applied identically everywhere, measured
something sociological: the fraction of ranked matches disrupted by a
leaver or AFK falls monotonically up the ladder —

| Bronze | Silver | Gold | Platinum | Diamond | Champion | GC |
|--:|--:|--:|--:|--:|--:|--:|
| 23% | 23% | 14% | 11% | 10% | 6.4% | 3.6% |

(~6× from bottom to top; the residual mix also shifts from rage-quits
toward brief AFKs as rank rises.)

## What to practice, per bracket (each entry's #1–2, distilled)

| at | practice this | because |
|---|---|---|
| **Bronze** | Boost basics + stop chasing when your teammate is on the ball | 5 metrics below the whole ladder's floor; 91% share one leak |
| **Silver** | Spend less boost for the same movement; start the fast aerial | Boost churn replaces starvation; the air gap starts pricing rank |
| **Gold** | Bank a mid-tank; treat "don't double-commit" as the rank-up mechanic | Discipline is Gold's strongest divisional separator |
| **Platinum** | Turn aerial *attempts* into air *presence*; scan on a schedule | 5.2 aerial touches/match but 4% high-air time; facing-ball now tracks division |
| **Diamond** | Own the second air touch; recover like it's a mechanic | Air dribbling's biggest jump; landings start separating divisions |
| **Champion** | Make possession a discipline; tighten the pair | Retention is the new frontier; tighter spacing now correlates higher |
| **GC** | Survive the lean tank; the air game *still* isn't finished | Aerial +0.223 / boost +0.172 — the series' strongest divisional signals |

## Where the rubric's validity ends

Two boundaries surfaced, honestly:

1. **At the top, the textbook stops describing the game.** GC's falling
   second-man sub-score, "regressing" double-commits, "too-tight" spacing,
   and flat-to-down transition/challenge metrics are one coherent
   phenomenon: the mid-ladder role template reads deliberate elite style
   as leak. Above Champion, within-bracket assessment should lean on the
   rank-relative layer (`scoring::relative`) and the value model's ΔV
   rather than the absolute composite.
2. **Within-bracket correlations are range-restricted everywhere** — the
   reliable cross-bracket signal is each bracket's position on the
   corpus-wide calibrated curves, not tier-vs-metric Spearman inside a
   3-tier slice (each entry carries this caveat with its numbers).

## The recalibration (post-series)

With every bucket's replays local for the first time (1,055 of 1,057
files; 2 deleted upstream), the committed corpus artifacts were
regenerated on the full Bronze→SSL corpus — the first calibration in the
repo's history to include the bottom of the ladder:

- **Value model** (`value_model.json`): retrained on 1,055 replays /
  676k grid rows (~6× the original corpus). GBT VAL AUC **0.730** vs
  logistic 0.716 on the held-out split (the previous 0.741 was measured on
  the narrower Silver→GC population — not directly comparable; the GBT
  still clearly wins the head-to-head and beats the base rate).
- **Scoring rubric** (`fitted_config.json`, → `scfg-v13-fitted+promoted`):
  curves and weights refit to rank on the seven-bucket corpus. Headline
  cross-validated composite-vs-rank Spearman **ρ = 0.809** (ridge weights)
  — holding the previous 0.806 while extending the ruler down two
  brackets, with cleanly monotone fitted per-rank composite means
  (Bronze 32.9 → Silver 40.1 → Gold 44.3 → Plat 50.2 → Diamond 55.8 →
  Champion 59.0 → GC 62.1). The reconcile pass found **zero sign
  disagreements** between the rank and ΔV ground truths, and graduated two
  candidates on in-match-impact evidence: `pace` (ρ_ΔV +0.159, w = 0.025)
  and `agility` (+0.119, w = 0.014); `dangerous_turnover` stays
  experimental (no ΔV signal).
- **Rank-relative norms** (`rank_norms.json`, now committed): built with
  **all seven buckets** — the rank-relative layer (`RelativeReport`, the
  "for your level" coaching view, and the app's "vs rank" view) can place
  a player against genuine Bronze or Silver peers for the first time.
  Previously this artifact wasn't committed at all, so the app always fell
  back to absolute-only scoring.

The per-bracket write-ups in this series were deliberately produced with
the *pre-recalibration* config so all seven are mutually comparable; their
numbers describe that fixed ruler and remain valid as published.
