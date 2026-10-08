# Rank assessment — Platinum

Fourth entry in the per-rank series
([Bronze](rank-assessment-bronze.md) · [Silver](rank-assessment-silver.md) ·
[Gold](rank-assessment-gold.md)): what Platinum play looks like in the replay
data, where it sits on the full ladder, what changed since Gold — same
tooling on all four brackets — and what closes the gap to Diamond.

## Data note

**128 replays** already in the manifest, all legacy full-ranked entries.
Unlike Gold, 7 lobbies contain a borderline out-of-band player (5 Gold III
slots, 4 Diamond I — mean-tier bucketing admits near-boundary lobbies); the
player-level tier filter excludes those 9 individuals while keeping their
in-band lobby-mates, and 6 of them survived to the confidence-clean stage
before being dropped. Division balance is good (145 Plat I / 102 II / 183
III among clean observations).

**Coverage gate:** 14 of 128 replays (11%) — the decline continues
(Bronze 23% → Silver 23% → Gold 14% → Platinum 11%). 11 involve a mid-match
leaver (2 of whom rejoined, fragmenting into extra tracks; 3 idled ≥20s
before quitting), 3 are pure AFK-while-connected — including one replay
where **two players on opposite teams** idled simultaneously mid-match. All
14 verified individually against lobby-mates' speeds and kickoff/goal dead
time; no false positives.

**Every number below uses the 114 clean replays — 430 confident,
in-bracket player-observations**, scored through the shipped, unchanged
`fitted_config.json`. As before, within-bracket correlations come from an
isolated scratch manifest; no committed calibration artifact was refit.

## Headline: the climb re-accelerates

| | Platinum | Gold | Silver | Bronze |
|---|---|---|---|---|
| Composite | **43.7** / 42.8 med, σ=10.5 | 35.7 / 35.1 | 30.1 / 28.3 | 21.7 / 20.1 |
| Quartiles | p25 36.0 · p75 51.0 | 28.1 · 42.0 | 21.5 · 36.3 | 13.9 · 26.4 |
| First-man | 50.9 | 45.7 | 37.5 | 28.9 |
| Second-man | 56.8 | 47.9 | 46.1 | 36.9 |
| **General fundamentals** | **38.5** | 30.6 | 23.8 | 15.8 |
| Modal licence bands | Gold + Platinum (33%/30%) | Gold | Silver/Unranked | Unranked |

The bracket-over-bracket increment **re-accelerates**: +8.4 (B→S), +5.6
(S→G), **+8.0 (G→P)** — revising Gold's "diminishing returns" reading. The
Silver→Gold step was the small one; the ladder's difficulty isn't a smooth
curve. Composite is monotonic within the bracket (Plat I 40.7 → II 44.5 →
III 45.6), and the licence distribution straddles Gold/Platinum as its two
modal bands — fourth consecutive agreement between the absolute composite
and the real bracket, now with the distribution centered right where the
bracket sits.

Ladder positions, all four brackets (weighted metrics, ordered by Platinum):

| metric | weight | Bronze | Silver | Gold | Platinum |
|---|--:|--:|--:|--:|--:|
| **aerial_presence** | 0.042 | 16.2% | 18.7% | 25.6% | **33.2%** |
| **boost_management** | 0.054 | −3.8% | 11.8% | 26.3% | **33.5%** |
| pace | 0.012 | −5.6% | 16.8% | 29.9% | 39.9% |
| reverse_driving | 0.036 | 7.4% | 20.0% | 26.7% | 41.6% |
| ball_chase_index | 0.002 | −1.7% | 25.3% | 34.7% | 43.6% |
| facing_ball_share | 0.017 | −6.5% | 20.0% | 32.5% | 44.5% |
| possession_retention | 0.000 | 58.1% | 52.2% | 48.0% | 44.8% |
| goalside_discipline_1st | 0.000 | 19.4% | 32.7% | 43.0% | 46.9% |
| recovery_speed | 0.005 | 45.5% | 51.1% | 50.5% | 48.3% |
| goalside_discipline_team | 0.000 | 14.0% | 30.2% | 43.3% | 48.8% |
| first_touch_value | 0.002 | 28.4% | 40.8% | 50.3% | 50.0% |
| overcommit_rate | 0.007 | 3.4% | 28.6% | 41.6% | 50.6% |
| central_support_fraction | 0.030 | 41.3% | 43.7% | 42.4% | 51.3% |
| challenge_timing | 0.000 | 36.3% | 41.6% | 49.1% | 52.2% |
| transition_readiness | 0.000 | 32.7% | 41.3% | 45.1% | 52.2% |
| boost_starvation | 0.002 | −4.2% | 23.6% | 41.1% | 53.1% |
| double_commit_rate | 0.019 | 4.9% | 31.0% | 38.7% | **57.7%** |
| support_spacing (band) | 0.008 | 2674 uu | 2970 | 2984 | **3112 uu** |

For the **fourth bracket running**, the two lowest-positioned weighted
metrics are the two highest-weighted ones — `boost_management` and
`aerial_presence`. Everything else has crossed the ladder's midpoint or sits
near it; these two are half a ladder behind the bracket's own discipline
stats. `double_commit_rate` (57.7%) is now Platinum's *best* metric — the
Gold-era lesson fully absorbed — and `support_spacing` lands almost exactly
on the fitted band's center (3065 uu): textbook 2v2 spacing is achieved at
Platinum.

## The leaks, fourth verse: boost and air, ever more concentrated

`main_leak = boost_management` for **301 of 430 (70%)**, `aerial_presence`
for **102 (24%)** — together 94% of all Platinum leaks, the most
concentrated leak profile of any bracket yet (`reverse_driving` collapses to
6%, everything else to noise). The ladder's message to Platinum is
remarkably specific: *manage boost, own the air*.

The boost paradox sharpens once more: collection reaches 327 bpm (+6% over
Gold, +44% over Bronze) and spend 314 bcpm, yet the average gauge creeps up
just one point (50.8 → 52.0) and time-at-full keeps *falling* (15.4% →
14.7%). Even at Platinum, boost is a flow resource, not a stored one.
Within-bracket its Spearman is +0.054 — like Gold, it no longer separates
neighbors, only brackets.

## Within-Platinum: the separators move to perception and possession

The within-bracket signal rotates again (each bracket's top separators are
the *next* skill frontier, and they keep changing):

1. `reverse_driving` **−0.146** — the last car-control tell; still the
   single strongest habit separator inside Platinum.
2. `facing_ball_share` **−0.133** — *scanning* now separates neighbors:
   Plat IIIs look away from the ball measurably more than Plat Is. The
   bracket-level trend (44.5% ladder position) is finally being driven from
   within.
3. `challenge_timing` **+0.114** — carried over from Gold's separators.
4. `boost_starvation` **−0.110** — not how much boost flows, but avoiding
   the stranded-at-zero runs.
5. `possession_retention` **+0.108** — the inflection: bracket-over-bracket
   retention *keeps falling* (58 → 52 → 48 → 45, four brackets of trading
   touch quality for tempo), yet **inside** Platinum, the players who keep
   the ball rank higher. This is the first bracket where possession is a
   within-bracket virtue — the tempo trade has gone as far as it profitably
   can.

Gold's headline separator, `double_commit_rate`, is spent (−0.019): the
discipline is now bracket-wide (and its ladder position, 57.7%, is
Platinum's best stat). `aerial_presence` +0.081 keeps a mild edge.

## What changed on the field, Gold → Platinum

**Boost:** bpm 310 → 327, bcpm 299 → 314, gauge 50.8 → 52.0, time-at-zero
14.2% → 12.7%, steals 331 → 345. Same shape, more volume.

**Movement:** the powerslide count **explodes 18.5 → 29.2 per match**
(+58%; Bronze was 7 — Platinum slides 4× as often). Average speed +31 uu/s;
high-air time finally moves meaningfully in relative terms, 3.8% → 4.2%
(+12%), the first bracket where the *high* air budget grows faster than the
low. Ground time 69.2% → 68.5%.

**Positioning: four brackets, no movement.** Defensive-third 49.2%,
behind-ball 73.7%, most-back/most-forward ~50/50, last-defender concessions
1.41/match — every one statistically where Bronze left it. The single
positioning stat that keeps improving is teammate spacing (+117 uu, now
centered on the fitted band). Where players stand relative to the *field*
hasn't changed since Bronze; where they stand relative to *each other* has
quietly become correct.

**Demos:** first sign of life — 0.73 inflicted/match (up from the
0.6-flatline of the three brackets below), 41% of players landing one.

## Mechanics: aerial volume soars; the flick dips; control creeps

| skill | Bronze | Silver | Gold | Platinum | rate/match (Plat) |
|---|--:|--:|--:|--:|--:|
| aerial | 67% | 72% | 93% | **97%** | **5.23** |
| redirect | 63% | 85% | 93% | 96% | 3.61 |
| power_shot | 68% | 85% | 92% | 92% | 3.17 |
| boost_steal | 74% | 75% | 84% | 87% | 2.22 |
| wall_play | 56% | 56% | 69% | **74%** | 1.60 |
| ground_dribble | 11% | 13% | 18% | **24%** | 0.30 |
| air_dribble | 6% | 9% | 11% | 17% | 0.20 |
| ceiling_play | 33% | 36% | 45% | 41% | 0.55 |
| flick | 53% | 59% | 57% | **51%** | 0.72 |
| double_touch | 13% | 8% | 10% | 12% | 0.12 |

Aerial *volume* is the bracket's mechanical signature: 5.23 touches/match,
+35% over Gold and 2.6× Bronze — yet `aerial_presence` (sustained air
threat) still sits at 33% of the ladder and is the #2 leak. Platinum goes up
constantly and briefly. The control wave finally shows a pulse — ground
dribble 18% → 24%, air dribble 11% → 17% — though rates stay tiny
(0.2–0.3/match). And one curiosity: the **flick declines** (59% → 57% → 51%
across S→G→P) — mid-ladder offense consolidates around power shots and
redirects, and the pop-flick apparently gets *deprioritized* before the
dribble game that makes it dangerous has matured.

## The limits of Platinum, synthesized

1. **Boost and air are now 94% of the problem.** The most concentrated leak
   profile of any bracket: everything else has converged to mid-ladder;
   these two (also the two highest-weighted metrics) lag half a ladder
   behind the bracket's own discipline stats — the fourth consecutive
   bracket with the same two names at the bottom of the table.
2. **Air *volume* without air *presence*.** 97% of players, 5.2 aerial
   touches/match, high-air time 4.2%. Platinum jumps for everything and
   lives in the air for nothing — the sustained-threat game (position →
   commitment → follow-up in the air) is what Diamond prices.
3. **Field positioning is a four-bracket fossil.** Defensive-third,
   behind-ball, last-defender numbers unchanged since Bronze. What improved
   is pair geometry (spacing now textbook). The next positioning gains have
   to come from *reading* — being goalside of the right threat earlier —
   not from standing zones.
4. **The tempo trade has maxed out.** Retention fell four brackets straight,
   but inside Platinum it now *positively* separates players. Faster stopped
   being better; cleaner is starting to be.
5. **Scanning is the new frontier habit.** Facing-ball share is a top-2
   within-bracket separator for the first time — at Platinum, where your
   camera points measurably tracks your division.

## How to improve beyond Platinum

1. **Turn aerial volume into air presence.** You already jump 5+ times a
   match; the upgrade is *staying relevant while airborne* — double-jump
   aerials with a second touch in mind, wall-reads that end in controlled
   air touches (wall play is already 74% and growing), and picking the high
   balls you can *own*, not just poke. This is the #2 leak, the #2 gap to
   the ladder, and the clearest Diamond toll.
2. **Make the gauge a battery, not a pipe.** Time-at-full is *falling* as
   collection rises — Platinum instantly converts pickup into motion. The
   drill: hold 60–100 through neutral phases and *choose* the moment to
   spend it (a committed aerial, a demo run, a fast recovery), rather than
   bleeding it into cruising speed. Boost management is the #1 leak for the
   fourth straight bracket; at this level it's about *when*, not *whether*,
   you collect.
3. **Keep the ball once you have it.** Possession retention is the new
   positive separator inside the bracket — and the dribble mechanics that
   express it are finally budding (24% ground, 17% air). This is the
   bracket where carry-and-flick practice starts paying rank — and note the
   flick has *declined* to 51% of players; reviving it on top of a dribble
   makes it a finisher rather than a pop.
4. **Scan on a schedule.** Facing-ball −0.133 within the bracket: build the
   explicit habit — every neutral touchline moment, glance at boost,
   teammate, and the far post. Platinum is where this measurably shows up
   in division.
5. **Trust the discipline you've built.** Double-commits (your best stat),
   overcommits, goalside shape — all at or past mid-ladder now. The
   remaining wins are additive (air, boost, possession), not corrective;
   don't regress the structure chasing the new toys.

## Reproducing / extending this analysis

```sh
BC_TOKEN=<token> python assets/corpus/refresh_corpus_replays.py --bucket platinum
python assets/corpus/refresh_manifest_playlist.py --skip-missing

./target/release/replay-scoring --config assets/corpus/fitted_config.json --json out.json <replay>
./target/release/replay-analyzer --bc-stats out.json <replay>
./target/release/replay-skills --json out.json <replay>
# filter to "confidence": "ok" and manifest tier 10..12
```

As with all prior brackets, `calibrate` was **not** run against the full
manifest; within-Platinum numbers come from an isolated scratch manifest,
and the committed `fitted_config.json`/`value_model.json` are byte-identical
to before.
