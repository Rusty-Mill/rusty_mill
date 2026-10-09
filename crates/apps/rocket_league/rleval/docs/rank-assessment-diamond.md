# Rank assessment — Diamond

Fifth entry in the per-rank series
([Bronze](rank-assessment-bronze.md) · [Silver](rank-assessment-silver.md) ·
[Gold](rank-assessment-gold.md) · [Platinum](rank-assessment-platinum.md)):
what Diamond play looks like in the replay data, where it sits on the full
ladder, what changed since Platinum — same tooling on all five brackets —
and what closes the gap to Champion.

## Data note

**197 replays** (the corpus's second-largest bucket), all legacy full-ranked
entries. 13 lobbies carried a borderline out-of-band player; the player-level
tier filter dropped the 16 such individuals that survived to the clean stage
(13 Champion-I, 3 Platinum) while keeping their in-band lobby-mates.
Division balance: 105 Diamond I / 216 II / 339 III among clean observations
(skewed high like most buckets).

**Coverage gate:** 20 of 197 (10%) — the abandonment slide continues
(23% → 23% → 14% → 11% → 10%). 13 involve leavers (2 rejoins, one lobby
losing *two* players, and one 55-second fragment where all four tracks end
together — an aborted match the span check correctly rejects as
unrepresentative), 7 are pure AFK-while-connected (one replay with two
players idle simultaneously). All
verified individually; no false positives.

**Every number below uses the 177 clean replays — 660 confident, in-bracket
player-observations**, scored through the shipped, unchanged
`fitted_config.json`. Within-bracket correlations from an isolated scratch
manifest; no committed artifact refit.

## Headline: the bottleneck breaks

| | Diamond | Platinum | Gold | Silver | Bronze |
|---|---|---|---|---|---|
| Composite | **55.8** / 55.6, σ=10.8 | 43.7 | 35.7 | 30.1 | 21.7 |
| Quartiles | p25 47.9 · p75 63.4 | 36.0 · 51.0 | 28.1 · 42.0 | 21.5 · 36.3 | 13.9 · 26.4 |
| First-man | 56.6 | 50.9 | 45.7 | 37.5 | 28.9 |
| Second-man | 59.9 | 56.8 | 47.9 | 46.1 | 36.9 |
| **General fundamentals** | **54.2** | 38.5 | 30.6 | 23.8 | 15.8 |
| Modal licence bands | Platinum + Diamond | Gold + Platinum | Gold | Silver/Unr. | Unranked |

- **+12.1 composite — the largest bracket step of the entire series**
  (+8.4, +5.6, +8.0, +12.1). A bottom-quartile Diamond (47.9) out-scores the
  *median* Platinum (42.8).
- The jump is driven almost entirely by **general fundamentals** (+15.7,
  vs +5.7/+3.1 for the role scores) — which is exactly where the two
  four-bracket laggards lived. And indeed:
- **The bottleneck breaks.** `boost_management` leaps 33.5% → **51.2%** and
  `aerial_presence` 33.2% → **51.4%** — each crossing the ladder's midpoint
  in a single bracket after four brackets stuck at the bottom of the table.
  For the first time since Bronze, they are *not* the two lowest-positioned
  weighted metrics.
- The licence distribution straddles Platinum/Diamond as its modal pair —
  fifth consecutive agreement with the real bracket — and the rubric's top
  band (*Pacifist Master*) appears in volume for the first time (73 players,
  11%).

Full five-bracket ladder table (ordered by Diamond position — note how
**compressed** it has become; every metric now sits between 48% and 68%):

| metric | weight | Bronze | Silver | Gold | Plat | Diamond |
|---|--:|--:|--:|--:|--:|--:|
| possession_retention | 0.000 | 58.1% | 52.2% | 48.0% | 44.8% | **48.1%** |
| transition_readiness | 0.000 | 32.7% | 41.3% | 45.1% | 52.2% | 50.4% |
| **boost_management** | 0.054 | −3.8% | 11.8% | 26.3% | 33.5% | **51.2%** |
| **aerial_presence** | 0.042 | 16.2% | 18.7% | 25.6% | 33.2% | **51.4%** |
| central_support_fraction | 0.030 | 41.3% | 43.7% | 42.4% | 51.3% | 51.5% |
| recovery_speed | 0.005 | 45.5% | 51.1% | 50.5% | 48.3% | 51.6% |
| goalside_discipline_1st | 0.000 | 19.4% | 32.7% | 43.0% | 46.9% | 52.0% |
| challenge_timing | 0.000 | 36.3% | 41.6% | 49.1% | 52.2% | 52.7% |
| first_touch_value | 0.002 | 28.4% | 40.8% | 50.3% | 50.0% | 53.8% |
| overcommit_rate | 0.007 | 3.4% | 28.6% | 41.6% | 50.6% | 57.2% |
| goalside_discipline_team | 0.000 | 14.0% | 30.2% | 43.3% | 48.8% | 57.9% |
| reverse_driving | 0.036 | 7.4% | 20.0% | 26.7% | 41.6% | 58.3% |
| pace | 0.012 | −5.6% | 16.8% | 29.9% | 39.9% | 58.7% |
| ball_chase_index | 0.002 | −1.7% | 25.3% | 34.7% | 43.6% | 59.4% |
| boost_starvation | 0.002 | −4.2% | 23.6% | 41.1% | 53.1% | 61.6% |
| facing_ball_share | 0.017 | −6.5% | 20.0% | 32.5% | 44.5% | 63.3% |
| double_commit_rate | 0.019 | 4.9% | 31.0% | 38.7% | 57.7% | 67.9% |
| support_spacing (band) | 0.008 | 2674 | 2970 | 2984 | 3112 | 3193 uu |

Two threads resolve here:

- **`possession_retention` upticks (+3.3)** — its first rise after four
  brackets of monotone decline, cashing out exactly the inflection the
  Platinum assessment identified (retention turned into a positive
  within-Platinum separator; one bracket later the bracket-level number
  turns). It is now, however, the *lowest*-positioned metric on the sheet —
  the next thing the ladder above will demand.
- `support_spacing` (3193 uu) pushes past the fitted band's center toward
  its upper edge (3305): Diamond pairs play *wider* than textbook — spacing
  for the faster, longer plays this bracket generates.

## The leaks: same two names, but now they're half-solved

`main_leak = boost_management` for **395 of 660 (60%)** and
`aerial_presence` for **184 (28%)** — 88% combined. The names haven't
changed in three brackets; what changed is the level: both metrics now sit
*past the ladder's midpoint* while remaining the leaks, because the ladder
above Diamond keeps pricing them steeply. And within the bracket they are
**still the top separators** — `aerial_presence` **+0.157** (the #1
within-Diamond rank signal) and `boost_management` **+0.124** — so unlike
Gold/Platinum (where their within-bracket power was spent), at Diamond the
air and boost games separate divisions *and* brackets simultaneously. The
climb through Diamond *is* the air-and-boost climb.

## Within-Diamond: the air era, and landings start to count

1. `aerial_presence` **+0.157** — the strongest within-bracket separator of
   any metric in any bracket so far except Silver's boost signal.
2. `reverse_driving` **−0.136** — still shrinking, still separating (0.065
   mean — down ~40% from Bronze's 0.107).
3. `boost_starvation` **−0.128** / `boost_management` **+0.124** — both
   halves of the boost game.
4. `recovery_speed` **−0.100** — a first: *landing* quality enters the
   signal. Diamond's aerial volume (7.7 touches/match) makes the
   wheels-down-and-moving recovery a measurable edge.
5. `double_commit_rate` −0.086 — the discipline signal fading toward
   bracket-wide (its ladder position, 67.9%, is again the bracket's best).

Perception (facing-ball) — Platinum's frontier separator — is now flat
within-bracket (+0.015): scanning has become bracket-wide practice
(41.0% facing share, the series' lowest).

## What changed on the field, Platinum → Diamond

**Boost:** collection 327 → 361 bpm, spend 314 → 349 bcpm, and the gauge —
for the **fifth bracket running — sits at ~51** (46, 51, 51, 52, 51). The
five-bracket invariant is now unmissable: from Bronze to Diamond, players
roughly double the boost they move per minute and *never* raise the level
they hold. Diamond's new wrinkle is **denial**: stolen boost jumps +21% to
417/match (vs +4–13% at prior steps) — collection is becoming a weapon
pointed at the opponent, not just a fuel line. Time-at-zero even ticks *up*
(12.7% → 13.0%): spend growth outruns collection at the margin.

**Movement:** average speed +58 uu/s (1351); supersonic time 8.6% → 10.7%
(+25%); powerslides 29 → **43/match** (6× Bronze); ground time 68.5% →
65.7% with the freed time split between low air (+1.4pp) *and* — for the
first time meaningfully — **high air (4.2% → 5.6%, +33%)**.

**Positioning:** after four static brackets, one number finally moves —
defensive-third time **48.1%** (−1.1pp, shifted into the neutral third,
+1.1pp). Small, but it's the first field-position change of the series:
Diamond starts holding midfield rather than camping the back third.
Behind-ball (73.5%) and last-defender concessions (1.46) stay put; teammate
spacing widens another +72 uu.

**Demos:** 0.76/match, 43% of players — the slow creep continues; still not
a bracket weapon.

## Mechanics: the control era opens; the flick quietly dies

| skill | B | S | G | P | D | rate/match (D) |
|---|--:|--:|--:|--:|--:|--:|
| aerial | 67% | 72% | 93% | 97% | **100%** | **7.66** |
| redirect | 63% | 85% | 93% | 96% | 95% | 4.02 |
| power_shot | 68% | 85% | 92% | 92% | 95% | 3.69 |
| boost_steal | 74% | 75% | 84% | 87% | 90% | 2.80 |
| wall_play | 56% | 56% | 69% | 74% | **80%** | 1.85 |
| ceiling_play | 33% | 36% | 45% | 41% | 50% | 0.75 |
| **air_dribble** | 6% | 9% | 11% | 17% | **29%** | 0.42 |
| ground_dribble | 11% | 13% | 18% | 24% | 25% | 0.32 |
| double_touch | 13% | 8% | 10% | 12% | 15% | 0.17 |
| **flick** | 53% | 59% | 57% | 51% | **45%** | 0.59 |

Aerial touching is now universal (100% of players) and voluminous (7.7/match
— nearly 4× Bronze). The genuinely new development is **air dribbling
arriving as a real minority skill**: 17% → 29% of players (+71%), its
biggest jump of the series — multi-touch air control is where Diamond's
mechanical frontier sits, consistent with `aerial_presence` being the #1
within-bracket separator.

And one mechanic is going the *other* way: the **flick declines
monotonically from Silver** (59% → 57% → 51% → 45%). Mid-ladder offense
consolidated around power shots and redirects, and the pop-flick faded
before the dribble game that makes it dangerous matured (ground dribble has
merely crept 18% → 24% → 25%). The classic dribble→flick package — Champion
staple — is conspicuously *not yet assembled* at Diamond.

## The limits of Diamond, synthesized

1. **The air-and-boost climb is only half done.** Both broke through to
   ~51% of the ladder in one bracket, yet remain 88% of all main-leaks and
   the top two within-bracket separators. What Bronze→Platinum treated as a
   floor problem, Diamond→Champion treats as a ceiling problem: the same
   two names, now contested at midfield instead of the basement.
2. **Boost is a weapon but still not a battery.** Fifth straight bracket at
   a ~51 gauge; denial (+21% steals) is the new edge, but time-at-zero rose
   — Diamond outspends its own record collection rate. The stored-boost
   game (arriving at plays with a full commit available) is still unplayed.
3. **Possession is the new floor.** Retention's four-bracket decline ends
   with an uptick, but it's now the lowest-positioned metric on the sheet —
   and the mechanics that would express it (ground dribble stalled at 25%,
   flick at 45% and falling) haven't been assembled into a carry game.
4. **Field positioning has barely begun to move.** One point of
   defensive-third time shifted to midfield in five brackets;
   last-defender concessions unchanged since Silver. The reading-based
   positioning game remains almost entirely ahead.
5. **Landings are the hidden tax.** Recovery speed separates divisions for
   the first time — at 7.7 aerials/match, every slow landing compounds.
   Diamond plays in the air but hasn't learned to come *down* like the
   ladder above.

## How to improve beyond Diamond

1. **Finish the air game you started: own the second touch.** Air dribbling
   jumped 71% through this bracket (and has nearly tripled since Gold);
   aerial presence is the #1 division separator — the marginal hour goes to
   controlled double-touches and wall → air-dribble sequences, not more
   single aerial pokes (you already take 7.7/match).
2. **Recover like it's a mechanic — because it now measurably is.**
   Land wheels-down, nose toward the play, boost-line in view. Recovery
   speed is a within-Diamond separator for the first time; at Champion
   volume it compounds every exchange.
3. **Play the boost game on both sides of the ledger.** Diamond found
   denial (steals +21%); the unclaimed edge is *storage* — hold 60+ through
   neutral so the aerial you just trained is always available. Five
   brackets at a ~51 gauge says this habit, not collection rate, is the
   binding constraint.
4. **Assemble the carry→flick package.** Retention is the sheet's
   lowest-positioned metric, ground dribbling has stalled, and the flick
   has *declined* four brackets straight — the multi-touch ground game is
   the most under-built, highest-headroom mechanic set left. This is
   exactly what Champion lobbies punish you for lacking.
5. **Convert midfield presence into midfield *reads*.** The first
   field-position movement of the series (defensive third → neutral) has
   started; make it deliberate — hold midfield to intercept and pre-jump
   plays rather than to be closer to the ball. Behind-ball share and
   last-defender concessions are unchanged since Silver; those are the
   numbers a reading game moves.

## Reproducing / extending this analysis

```sh
BC_TOKEN=<token> python assets/corpus/refresh_corpus_replays.py --bucket diamond
python assets/corpus/refresh_manifest_playlist.py --skip-missing

./target/release/replay-scoring --config assets/corpus/fitted_config.json --json out.json <replay>
./target/release/replay-analyzer --bc-stats out.json <replay>
./target/release/replay-skills --json out.json <replay>
# filter to "confidence": "ok" and manifest tier 13..15
```

As with all prior brackets, `calibrate` was **not** run against the full
manifest; within-Diamond numbers come from an isolated scratch manifest, and
the committed `fitted_config.json`/`value_model.json` are byte-identical to
before.
