# Rank assessment — Gold

Third entry in the per-rank series (after
[`rank-assessment-bronze.md`](rank-assessment-bronze.md) and
[`rank-assessment-silver.md`](rank-assessment-silver.md)): what Gold play
looks like in the replay data, where it sits on the full ladder, what changed
since Silver — same tooling on all three brackets — and what closes the gap
to Platinum.

## Data note

No corpus work needed this time: Gold was already the third-largest bucket —
**123 replays**, all legacy full-ranked entries (every one of the 492
player-slots carries a tier, all strictly 7–9, zero out-of-bracket players).
One caveat: the bucket skews high (16 Gold I slots vs 204/272 for Gold
II/III), so per-division splits below are reliable for II/III and only
indicative for Gold I.

**Coverage gate:** 17 of 123 replays (14%) failed the lobby-completeness
gate — down from 23% at both Bronze and Silver, a small sociological finding
of its own: matches get abandoned less as rank rises. The mix also shifts:
10 involve a mid-match leaver (4 idled ≥20s first; 1 left and rejoined) but
**7 are pure AFK-while-connected** — proportionally much more than at Silver
(2/18), including one player frozen for 108 s and another for 80 s of an
otherwise normal match. All 17 were verified individually (each flagged
stretch cross-checked against the other players' simultaneous speeds and
kickoff/goal dead time); no false positives.

**Every number below uses the 106 clean replays — 415 confident
player-observations (11 Gold I / 174 Gold II / 230 Gold III)**, scored
through the shipped, unchanged `fitted_config.json`. As before, no
calibration artifact was refit; within-Gold correlations come from an
isolated scratch manifest.

## Headline: the era of specific pathologies is over

| | Gold | Silver | Bronze |
|---|---|---|---|
| Composite (0–100) | **mean 35.7**, median 35.1, σ=10.9 | 30.1 / 28.3 | 21.7 / 20.1 |
| Quartiles | p25 28.1 · p75 42.0 | 21.5 · 36.3 | 13.9 · 26.4 |
| First-man discipline | 45.7 | 37.5 | 28.9 |
| Second-man discipline | 47.9 | 46.1 | 36.9 |
| **General fundamentals** | **30.6** | 23.8 | 15.8 |
| Modal licence band | **Gold Licence (35%)** | Silver/Unranked | Unranked (70%) |

- Bracket-over-bracket gains are **shrinking**: +8.4 composite from Bronze to
  Silver, +5.6 from Silver to Gold. The easy wins (fixing outright
  pathologies) are used up.
- The rubric's own licence bands now **peak at Gold Licence for Gold
  players** — the third consecutive external agreement between the absolute
  composite and the real bracket.
- On the ladder-position table, the compression is the story: at Bronze,
  five weighted metrics sat *below* the entire ladder's calibrated floor; at
  Gold **no weighted metric is below 25%** of the ladder range. Gold isn't
  specifically broken anywhere — it's uniformly mediocre everywhere:

| metric | weight | Bronze | Silver | Gold |
|---|--:|--:|--:|--:|
| aerial_presence | 0.042 | 16.2% | 18.7% | **25.6%** |
| boost_management | 0.054 | −3.8% | 11.8% | **26.3%** |
| reverse_driving | 0.036 | 7.4% | 20.0% | 26.7% |
| pace | 0.012 | −5.6% | 16.8% | 29.9% |
| facing_ball_share | 0.017 | −6.5% | 20.0% | 32.5% |
| ball_chase_index | 0.002 | −1.7% | 25.3% | 34.7% |
| double_commit_rate | 0.019 | 4.9% | 31.0% | 38.7% |
| boost_starvation | 0.002 | −4.2% | 23.6% | 41.1% |
| overcommit_rate | 0.007 | 3.4% | 28.6% | 41.6% |
| central_support_fraction | 0.030 | 41.3% | 43.7% | 42.4% |
| goalside_discipline_1st | 0.000 | 19.4% | 32.7% | 43.0% |
| goalside_discipline_team | 0.000 | 14.0% | 30.2% | 43.3% |
| transition_readiness | 0.000 | 32.7% | 41.3% | 45.1% |
| challenge_timing | 0.000 | 36.3% | 41.6% | 49.1% |
| first_touch_value | 0.002 | 28.4% | 40.8% | 50.3% |
| recovery_speed | 0.005 | 45.5% | 51.1% | 50.5% |
| possession_retention | 0.000 | 58.1% | 52.2% | 48.0% |

(The two lowest-positioned metrics at Gold are also the two
highest-weighted — `aerial_presence` and `boost_management` — which is
exactly what the leak counter says below.)

One metric declines monotonically across all three brackets:
`possession_retention` (58.1% → 52.2% → 48.0%). Read with the pace/shot data
it isn't players getting *worse* — it's play getting faster and harder-hit
(power shots nearly double from Bronze to Gold), which trades first-touch
retention for tempo through these brackets.

## The leaks: boost still #1, the aerial gap now impossible to ignore

`main_leak = boost_management` for **283 of 415 (68%)** — third bracket in a
row, though its share keeps shrinking (91% → 79% → 68%). What's growing into
the space: **`aerial_presence`, now the main leak for 90 players (22%)** —
doubled from Silver's 13% — and `reverse_driving` (40, 10%).

The boost problem has changed shape again. Bronze's version was "runs to
zero and sits there"; Silver's was churn. Gold *intensifies* the churn:
collection jumps another +46 bpm and spend +48 bcpm (both ~+18%), while the
average gauge stays **flat at ~51** and time-at-full actually falls. Gold
players move more boost through the tank per minute than any bracket below
them and bank none of it. Within the bracket, boost management's separating
power is now exhausted (Spearman vs tier just +0.026 — every Gold is roughly
equally mediocre at it); it separates *brackets*, no longer neighbors. The
within-bracket separators have moved on (next section).

## Within-Gold: discipline starts paying — the Silver prediction confirmed

The Silver assessment flagged that double-commits *rise* with tier inside
Silver (+0.174 — eagerness before discipline) and predicted the corpus-wide
direction would reassert itself above that bracket. It does, immediately:
**inside Gold, `double_commit_rate` is the strongest within-bracket
separator at −0.185** — fewer double-commits, higher tier. The rest of the
within-Gold signal tells the same story of *structure* taking over from
*motor*:

1. `double_commit_rate` **−0.185** — the discipline flip.
2. `support_spacing` **+0.161** (band) — proper 2nd-man distance starts
   separating players for the first time.
3. `challenge_timing` **+0.117** — when you challenge, not just whether.
4. `reverse_driving` **−0.117** — the last car-control crutch fading.

Meanwhile the Silver-era separators are spent within-bracket:
`boost_management` +0.026, `aerial_presence` +0.025, `pace` +0.059 — these
still separate Gold *from Silver*, but no longer Gold I from Gold III.
(Composite-vs-tier inside Gold is nearly flat overall, +0.015 — restricted
range plus the 11-player Gold I sample; read the per-metric signs, not
magnitudes.)

## What changed on the field, Silver → Gold

**Boost:** bpm 263 → 310, bcpm 250 → 299, gauge flat (51.2 → 50.8),
time-at-zero 15.2% → 14.2%, steal volume +13% (294 → 331 — more of the
collection is happening in the opponent's half).

**Movement:** average speed 1221 → 1262 uu/s; slow-time 57% → 55%;
ground-time 72.5% → 69.2% with the difference going to **low** air (24.0% →
27.1%) — not high air (3.5% → 3.8%). **Powerslides 10.7 → 18.5 per match**
(+74%, and ~2.6× Bronze) — the turn mechanic Bronze lacked is now routine.

**Positioning:** essentially static — defensive-third ~49%, behind-ball
~72–73%, most-back/most-forward still ~50/50, goals-against-as-last-defender
flat (1.46 → 1.44), teammate spacing flat. **Where Gold stands is where
Silver stands**; everything that improved is in how they move, spend, and
hit. The positioning game that the next brackets are built on hasn't started
moving yet.

**Demos:** 0.68/0.68 inflicted/taken — still a wash, three brackets running.

## Mechanics: the shot-quality wave completes; the control wave hasn't begun

| skill | Bronze | Silver | Gold | rate/match (Gold) |
|---|--:|--:|--:|--:|
| aerial | 67% | 72% | **93%** | 3.88 |
| redirect | 63% | 85% | **93%** | 3.33 |
| power_shot | 68% | 85% | **92%** | 2.88 |
| boost_steal | 74% | 75% | 84% | 2.07 |
| wall_play | 56% | 56% | **69%** | 1.33 |
| ceiling_play | 33% | 36% | 45% | 0.61 |
| ground_dribble | 11% | 13% | 18% | 0.21 |
| air_dribble | 6% | 9% | 11% | 0.13 |
| double_touch | 13% | 8% | 10% | 0.12 |

The single-contact offensive game is now near-universal: 92–93% of Gold
players land aerial touches, redirects, and power shots, at roughly double
Bronze's per-match rates. Two new fronts open at Gold: **wall play** (56% →
69%, first movement since Bronze) and deliberate **boost denial** (84%
stealing, volume +13%). But sustained ball control still hasn't arrived —
ground dribble 18%, air dribble 11%, double touch 10%, all at ~0.1–0.2
attempts/match. Three brackets in, offense remains single-contact; the ball
is hit, never *carried*.

The aerial picture deserves its precision: Gold *attempts* aerials
near-universally (93%, 3.88/match) yet `aerial_presence` — time spent as a
credible air threat — sits at only 25.6% of the ladder range, is the #2 leak
(22%), and high-air time is still 3.8% of the match. Gold jumps for balls;
it does not yet *play* in the air.

## The limits of Gold, synthesized

1. **Boost flows through the tank but never accumulates.** Highest-weighted
   fundamental, still the #1 leak (68%), and the bracket's signature stat is
   churn: +18% collection *and* +18% spend over Silver with a dead-flat ~51
   gauge. Gold funds constant motion, not options — no banked boost means no
   spontaneous aerial, no sustained pressure, no fast recovery when the play
   flips.
2. **The air game is attempted, not possessed.** 93% of players make aerial
   touches; the bracket still spends 96% of the match below high-air height,
   and being an actual air threat is now the #2 leak and doubled its share.
   The ladder above prices air presence; Gold pays that toll next.
3. **Positioning has not moved in two brackets.** Defensive-third,
   behind-ball, most-back/most-forward, last-defender concessions, spacing —
   all statistically where Silver (and mostly Bronze) left them. Gold's
   gains are motor and mechanical; the *standing in the right place earlier*
   game is untouched, and it's what the within-bracket separators
   (`support_spacing`, `challenge_timing`) say is starting to matter.
4. **Discipline is now the active margin.** The double-commit flip
   (+0.174 inside Silver → **−0.185** inside Gold) marks this bracket as
   where restraint starts winning games — the first bracket where doing
   *less* is measurably how you rank up within it.
5. **Still no manufactured offense.** All multi-touch control mechanics
   remain at ≤18% of players and ~0.2 attempts/match. Gold scores off the
   first touch or not at all.

## How to improve beyond Gold

1. **Bank boost instead of cycling it.** The concrete habit: arrive at
   defensive positioning with ≥50 in the tank and *keep* it there — feather,
   don't hold; take small pads on rotation paths instead of detouring to big
   ones you'll immediately spend. The goal state is a gauge that spends time
   in the 50–100 band (Gold's time-at-full actually *fell* vs Silver), not a
   faster empty-full-empty cycle.
2. **Convert aerial attempts into air presence.** Gold already jumps for
   everything (3.88 aerial touches/match); the gap is height, timing, and
   comfort — fast-aerial mechanics into high-air balls, wall-reads that end
   airborne (wall play is already growing, 69%), and taking the uncontested
   high clear every time. This is the #2 leak and the next bracket's toll
   booth.
3. **Treat "don't double-commit" as the rank-up mechanic it now is.** It's
   the strongest within-Gold separator, in the direction Silver hadn't
   learned yet. The habit: when your teammate commits, your job description
   changes to *cover* — full stop.
4. **Start moving the positioning numbers that haven't moved since Bronze.**
   Two within-Gold separators name the work: keep second-man spacing in the
   2.8–3.3k band (not glued, not gone), and challenge on *timing* rather
   than proximity. The last-defender concession rate (~1.4/match, flat for
   two brackets) is the scoreboard for this work.
5. **Now start ground dribbling.** For the first time, the control mechanics
   tick up bracket-over-bracket (11→13→18% ground dribble) and everything
   cheaper is saturated: single-contact skills are at 90%+, so marginal
   practice-hours start favoring carry-and-flick over another power-shot
   rep. Air dribbles can still wait; dribble→flick is the Platinum-relevant
   piece.

## Reproducing / extending this analysis

```sh
# the gold bucket is already at 123 manifest entries; just fetch the files
BC_TOKEN=<token> python assets/corpus/refresh_corpus_replays.py --bucket gold
python assets/corpus/refresh_manifest_playlist.py --skip-missing

# score through the shipped (unchanged) production rubric; the
# lobby-completeness gate marks incomplete lobbies low-confidence automatically
./target/release/replay-scoring --config assets/corpus/fitted_config.json --json out.json <replay>
./target/release/replay-analyzer --bc-stats out.json <replay>
./target/release/replay-skills --json out.json <replay>
# then filter aggregation to reports where "confidence": "ok",
# and to players whose own manifest tier is 7..9
```

As with the previous brackets, `calibrate` was **not** run against the full
manifest (825 corpus files are still not downloaded locally; a full-manifest
run would refit the committed artifacts off only the local low-rank slice).
Within-Gold numbers come from an isolated scratch manifest; the committed
`fitted_config.json`/`value_model.json` are byte-identical to before.
