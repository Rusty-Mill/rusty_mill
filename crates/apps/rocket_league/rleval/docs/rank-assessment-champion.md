# Rank assessment — Champion

Sixth entry in the per-rank series
([Bronze](rank-assessment-bronze.md) · [Silver](rank-assessment-silver.md) ·
[Gold](rank-assessment-gold.md) · [Platinum](rank-assessment-platinum.md) ·
[Diamond](rank-assessment-diamond.md)): what Champion play looks like in the
replay data, what changed since Diamond — same tooling on all six brackets —
and what closes the gap to Grand Champion.

## Data note

**250 replays** — the corpus's largest bucket, all legacy full-ranked
entries. 27 lobbies carried a borderline out-of-band player; the player-level
tier filter dropped the 39 such individuals that reached the clean stage
(mostly GC-I tier 19 and Diamond-III tier 15) while keeping their in-band
lobby-mates. Division balance: 204 Champ I / 259 II / 332 III.

**Coverage gate:** 16 of 250 (**6.4%**) — the abandonment slide across the
series is now a clean monotone: 23% → 23% → 14% → 11% → 10% → 6%. Match
abandonment falls by ~4× from Bronze to Champion. Of the 16: 12 involve
leavers (3 of them rejoined, fragmenting into extra tracks), 4 are pure
AFK-while-connected (longest 54 s). All verified individually; no false
positives.

**Every number below uses the 233 clean replays — 795 confident, in-bracket
player-observations**, scored through the shipped, unchanged
`fitted_config.json`. Within-bracket correlations from an isolated scratch
manifest; no committed artifact refit.

## Headline: the fundamentals inversion completes

| | Champion | Diamond | Platinum | Gold | Silver | Bronze |
|---|---|---|---|---|---|---|
| Composite | **64.2** / 65.1 | 55.8 | 43.7 | 35.7 | 30.1 | 21.7 |
| Quartiles | p25 57.2 · p75 71.9 | 47.9 · 63.4 | 36.0 · 51.0 | 28.1 · 42.0 | 21.5 · 36.3 | 13.9 · 26.4 |
| First-man | 60.0 | 56.6 | 50.9 | 45.7 | 37.5 | 28.9 |
| Second-man | 60.7 | 59.9 | 56.8 | 47.9 | 46.1 | 36.9 |
| **General fundamentals** | **65.6** | 54.2 | 38.5 | 30.6 | 23.8 | 15.8 |
| Modal licence band | **Pacifist Master (33%)** | Plat+Diamond | Gold+Plat | Gold | Silver/Unr. | Unranked |

- **General fundamentals now *exceeds* both role scores** (65.6 vs
  60.0/60.7) — the complete inversion of Bronze, where fundamentals (15.8)
  trailed the role scores by half. Six brackets of ladder climbing were,
  in this rubric's terms, one long fundamentals catch-up — and it's done.
  What remains ahead is role craft.
- The rubric's **top band (Pacifist Master) is now the modal licence** (33%)
  — sixth consecutive external agreement, and an honest caveat with it: with
  one bracket still above, the absolute composite is nearing its calibrated
  ceiling, so expect compression (not another +8) when Grand Champion is
  assessed.
- Composite is monotone within the bracket (60.7 → 64.5 → 66.0).

The full six-bracket ladder table — note the **inversion of the old
order** (sorted by Champion position; the series' two chronic laggards are
now in the top half, and the *new* bottom of the table is the
possession/support-craft cluster):

| metric | weight | Bronze | Silver | Gold | Plat | Diamond | Champion |
|---|--:|--:|--:|--:|--:|--:|--:|
| **possession_retention** | 0.000 | 58.1% | 52.2% | 48.0% | 44.8% | 48.1% | **50.4%** |
| **central_support_fraction** | 0.030 | 41.3% | 43.7% | 42.4% | 51.3% | 51.5% | **51.1%** |
| transition_readiness | 0.000 | 32.7% | 41.3% | 45.1% | 52.2% | 50.4% | 51.4% |
| challenge_timing | 0.000 | 36.3% | 41.6% | 49.1% | 52.2% | 52.7% | 52.5% |
| first_touch_value | 0.002 | 28.4% | 40.8% | 50.3% | 50.0% | 53.8% | 53.0% |
| recovery_speed | 0.005 | 45.5% | 51.1% | 50.5% | 48.3% | 51.6% | 53.1% |
| goalside_discipline_1st | 0.000 | 19.4% | 32.7% | 43.0% | 46.9% | 52.0% | 57.3% |
| overcommit_rate | 0.007 | 3.4% | 28.6% | 41.6% | 50.6% | 57.2% | 62.3% |
| goalside_discipline_team | 0.000 | 14.0% | 30.2% | 43.3% | 48.8% | 57.9% | 62.7% |
| ball_chase_index | 0.002 | −1.7% | 25.3% | 34.7% | 43.6% | 59.4% | 63.1% |
| aerial_presence | 0.042 | 16.2% | 18.7% | 25.6% | 33.2% | 51.4% | **64.4%** |
| boost_starvation | 0.002 | −4.2% | 23.6% | 41.1% | 53.1% | 61.6% | 64.4% |
| boost_management | 0.054 | −3.8% | 11.8% | 26.3% | 33.5% | 51.2% | **67.1%** |
| facing_ball_share | 0.017 | −6.5% | 20.0% | 32.5% | 44.5% | 63.3% | 67.3% |
| double_commit_rate | 0.019 | 4.9% | 31.0% | 38.7% | 57.7% | 67.9% | 69.7% |
| pace | 0.012 | −5.6% | 16.8% | 29.9% | 39.9% | 58.7% | 70.6% |
| reverse_driving | 0.036 | 7.4% | 20.0% | 26.7% | 41.6% | 58.3% | 71.8% |
| support_spacing (band) | 0.008 | 2674 | 2970 | 2984 | 3112 | 3193 | **3173 uu** |

Boost management and aerial presence take *second consecutive* ~13–16-point
leaps (their two-bracket run: 33 → 51 → 65+) — the Diamond→Champion climb,
like Diamond's, is still substantially an air-and-boost climb. Meanwhile the
metrics that never had a growth spurt are now simply *last*:
possession retention, central support, transition readiness, challenge
timing, first-touch value — the craft of what you do *with* and *around*
the ball once the car control is solved. That cluster has been flat since
Platinum while everything else doubled.

Also note `support_spacing`'s reversal — the first contraction of the series
(3193 → 3173 uu), and *within* Champion, tighter spacing correlates with
higher tier (band Spearman −0.165, the strongest spacing signal of the
series). Diamond overspread for its long plays; Champion starts pulling back
toward the band. Spacing is dialectic, not monotone: too tight (Bronze) →
too wide (Diamond) → recompressing (Champion).

## The leaks: the duopoly finally weakens

`main_leak = boost_management` for **385 of 795 (48%)** — under half for the
first time in the series (91% → 79% → 68% → 70% → 60% → 48%). Aerial
presence 255 (32%), then a genuinely broadening tail: `reverse_driving` 81
(10%), `central_support_fraction` 38 (5%), `facing_ball_share` 35 (4%). As
the two chronic bottlenecks half-resolve, Champion's problems diversify —
the first bracket where the coaching prescription isn't nearly the same for
everyone.

Within-bracket, though, the same two still rule: `boost_management`
**+0.139** (its separating power *returns* after going flat in Gold/Plat —
at Champion, the boost game differentiates divisions again) and
`aerial_presence` **+0.115**. Third: `reverse_driving` −0.099. The
six-bracket truth is remarkably stable: whatever else changes, boost and air
decide who climbs.

## What changed on the field, Diamond → Champion

**Boost:** collection 361 → 386 bpm, spend 349 → 372 bcpm — and the average
gauge falls back to **50.0**, making it **six brackets inside 46–52** while
per-minute flow rose 70% from Bronze. Time-at-full hits its series *low* (12.9%)
and time-at-zero ticks up again (13.4%): Champion runs the leanest tank yet.
The boost economy the ladder rewards is unambiguously *throughput +
denial* (steals 443, +6%), not storage — revising the "bank it" hypothesis
from earlier write-ups: the data says the winning pattern is a fast-cycling
tank where what's banked is *field control of pads*, not gauge.

**Movement:** speed 1388 (+37), supersonic 12.8% (+20%), ground time 63.3%
(−2.4pp) with gains in both air bands (high-air 6.8%, +21%). Powerslides
49.7/match (7× Bronze).

**Positioning:** the reads finally move — **goals-against-as-last-defender
1.46 → 1.36**, the first meaningful drop of the series after five static
brackets; defensive-third time continues its slow midfield migration
(48.1% → 47.4%); spacing contracts (above). Behind-ball (73.8%) still
unchanged — the *shape* holds; what improved is what happens when the shape
is tested.

**Demos: the sleeping stat wakes.** 0.76 → **0.93/match (+22%)**, 48% of
players — after five brackets of ~0.6–0.76 flatline, demolitions become a
real part of the toolkit at Champion.

## Mechanics: air control becomes majority; the flick's fall continues

| skill | B | S | G | P | D | C | rate/match (C) |
|---|--:|--:|--:|--:|--:|--:|--:|
| aerial | 67% | 72% | 93% | 97% | 100% | 100% | **9.72** |
| redirect | 63% | 85% | 93% | 96% | 95% | 97% | 4.47 |
| power_shot | 68% | 85% | 92% | 92% | 95% | 97% | 4.04 |
| boost_steal | 74% | 75% | 84% | 87% | 90% | 93% | 3.00 |
| wall_play | 56% | 56% | 69% | 74% | 80% | **86%** | 2.09 |
| ceiling_play | 33% | 36% | 45% | 41% | 50% | **59%** | 0.96 |
| **air_dribble** | 6% | 9% | 11% | 17% | 29% | **47%** | 0.73 |
| ground_dribble | 11% | 13% | 18% | 24% | 25% | 31% | 0.41 |
| double_touch | 13% | 8% | 10% | 12% | 15% | 20% | 0.24 |
| **flick** | 53% | 59% | 57% | 51% | 45% | **40%** | 0.52 |
| demo | 31% | 37% | 36% | 41% | 43% | 48% | 0.93 |

Air dribbling jumps again (29% → 47%, +62%) — from a minority skill to
the cusp of majority in one bracket; ceiling play crosses half the
population; wall play hits 86%. The multi-touch *air* control era that
opened at Diamond is maturing fast. Ground control follows more slowly
(dribble 31%, double-touch 20%). And the **flick falls a fourth consecutive
bracket from its Silver peak** (59% → 40%): six brackets of data now say the
pop-flick is a low-rank mechanic that mid/high ladder progressively
abandons rather than refines — the carry game that arrives at
Diamond/Champion expresses itself through air dribbles and double touches
instead.

## The limits of Champion, synthesized

1. **The frontier has moved from car to ball.** Everything about *moving
   the car* (pace, slides, reversing, chase discipline, double-commits) now
   sits at 62–72% of the ladder; everything about *keeping and working the
   ball* (retention 50.4%, first-touch value 53.0%, challenge timing 52.5%,
   central support 51.1%, transition readiness 51.4%) is flat since
   Platinum and now defines the bottom of the table. Champion has solved
   locomotion; it has not solved possession.
2. **Boost and air still gate the divisions** — sixth bracket running.
   Both leapt ~15 ladder-points again, both remain the top two
   within-bracket separators and 80% of main-leaks. The climb to GC is more
   of the same, at higher altitude.
3. **The tank runs leanest here.** Six brackets inside a 46–52 gauge while
   flow rose 70%; Champion holds the least full-boost time of the series.
   The stored-boost hypothesis is dead — what separates the ladder is
   cycle rate and denial, and Champion's remaining gap is starvation
   avoidance under that lean regime (`boost_starvation` still a
   within-bracket signal at −0.050, leak #1 at 48%).
4. **The backline read arrived; the front line hasn't.** Last-defender
   concessions posted their first real drop, but behind-ball share and
   defensive-third time barely move: Champion defends better without yet
   *positioning* differently. Offensive-third time (21.6%) is unchanged
   across all six brackets — sustained offensive-zone pressure remains
   unplayed territory.
5. **Spacing enters its precision phase.** After five brackets of "wider is
   better", higher Champions are measurably *tighter* than lower ones —
   spacing stops being about escaping your teammate and starts being about
   arriving second at the right radius.

## How to improve beyond Champion

1. **Make possession a discipline.** The lowest-positioned metric on the
   sheet, flat for three brackets, and the thing the GC bracket above
   (previous corpus calibration: composite ~75+) does that you don't:
   first touches that keep options (first_touch_value has been flat since
   Gold), carries that survive a challenge, and transitions that don't give
   the ball straight back. Concretely: treat every clear as a *pass to
   future you* — corner placement over distance.
2. **Keep pushing air control toward universality.** Air dribbling at 47%
   and rising is the single fastest-moving skill on the ladder right now;
   aerial presence is still the #2 separator. The GC bar is multi-touch
   air plays as routine, not highlight.
3. **Run the lean tank without the stalls.** Champion already cycles boost
   at series-max rates; the divisional edge (`boost_management` +0.139,
   `boost_starvation` −0.050) is in *never being caught at zero in a live
   play* — pad-path discipline at full speed, small-pad economy in the
   opponent's half (steals are already 93%-adopted; make them routing, not
   raids).
4. **Tighten the pair.** The spacing signal flipped: at this level, the
   winning second man is *closer* than the bracket average — near enough to
   convert the first man's touch before the play resets. Practice arriving
   at the back-post radius, not the halfway line.
5. **Weaponize the demo game you just discovered.** +22% in one bracket and
   climbing; at GC it's a standard pressure tool. Fold demo threats into
   boost-steal routes — the two combine into the same opponent-half
   disruption runs.

## Reproducing / extending this analysis

```sh
BC_TOKEN=<token> python assets/corpus/refresh_corpus_replays.py --bucket champion
python assets/corpus/refresh_manifest_playlist.py --skip-missing

./target/release/replay-scoring --config assets/corpus/fitted_config.json --json out.json <replay>
./target/release/replay-analyzer --bc-stats out.json <replay>
./target/release/replay-skills --json out.json <replay>
# filter to "confidence": "ok" and manifest tier 16..18
```

As with all prior brackets, `calibrate` was **not** run against the full
manifest; within-Champion numbers come from an isolated scratch manifest, and
the committed `fitted_config.json`/`value_model.json` are byte-identical to
before.
