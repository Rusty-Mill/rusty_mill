# Rank assessment — Bronze

First entry in a per-rank series (`docs/rank-assessment-<rank>.md`) that asks,
per bracket: what does this rank's play actually look like in the replay data,
where does it sit against the full ranked ladder, and what specifically closes
the gap to the next bracket. Bronze is first because it didn't exist in this
repo's data at all until now — see **Data note** below.

## Data note: Bronze was not in the corpus

`assets/corpus/manifest.json` (996 replays before this write-up) spanned
`silver`..`grand-champion` only. `expand_manifest.py` excluded Bronze/unranked
by explicit design (`# Bronze/unranked excluded`), so every existing
calibration, the shipped `fitted_config.json`, and the rank-relative grading
layer (`scoring::relative`) have **never seen a Bronze replay**. This
assessment required pulling a first Bronze sample before it could say anything
evidence-based:

- Added a `bronze` bucket to `expand_manifest.py` (tiers 1–3, ballchasing slugs
  `bronze-1`..`bronze-3`) — purely additive, the other six buckets are
  untouched.
- Pulled **30 ranked-doubles Bronze replays** (the same per-tier size the
  corpus itself started at before growing to 250/tier) via the existing
  `expand_manifest.py` → `refresh_corpus_replays.py` → `refresh_manifest_playlist.py`
  pipeline. All 30 decode clean and backfill as genuine ranked-doubles 2v2 from
  their real headers, and every individual player tier in every replay is
  confirmed 1–3 (no smurf/mixed-rank contamination — checked directly against
  `manifest.json`'s per-replay `tiers` arrays; 9 of the 30 mix adjacent Bronze
  divisions, e.g. `[1,2,2,2]`, which is normal matchmaking, not a data error).
- `manifest.json` now carries these 30 entries (`bucket: "bronze"`,
  1026 total). **`fitted_config.json` and every other calibration artifact are
  untouched** — this write-up scores the new Bronze replays through the
  existing, already-shipped rubric rather than refitting it, so the numbers
  below are directly comparable to what any other rank gets from `rleval`
  today, and nothing else in the repo's calibrated state changed.

## Data-quality gate: was everyone actually there the whole game?

Before trusting the numbers, this needed a real answer, and the pipeline had
**no check for it anywhere** — not in `expand_manifest.py` (which only
verifies ballchasing lists 2+2 ranked players, nothing about whether they
played the whole match), and not in the Rust `Confidence` gate (which only
checked the *target* player's own frame count, never their teammates or
opponents).

Decoding all 30 replays and comparing each player's track span against the
match duration found **6 replays where a player disconnected partway**
(coverage as low as 22% — left after barely a minute of a 4.5-minute match)
and, on a closer pass checking for sustained near-zero-speed stretches (a
player who stays connected but stops playing — one is even named
`bakedAFK6683`), **1 more replay with a genuine ~112-second mid-match AFK
stretch** spread across four separate freezes. Cross-checking every flagged
stretch against the other three players' simultaneous speeds ruled out false
positives from goal-reset freeze-frames (where the whole lobby briefly stops
at once — not a real AFK) and from brief, single-player net-parking (a few
seconds of legitimate defensive holding, not quitting).

This matters beyond the leaver's own row: **a missing or AFK player turns part
of the match into an unrepresentative man-advantage/disadvantage for the other
three**, which the `Confidence` gate's own existing doc-comment already named
as a reason for low confidence (a missing/AFK teammate makes support metrics
meaningless) — it just never checked for it directly. Fixed at the source
(`scoring/src/coverage.rs`, wired into `scoring::score`'s confidence
computation): every track in the match — not just the scoring target — is
checked for (a) span ≥90% of match duration (catches a disconnect) and (b) no
continuous idle stretch >20s outside kickoff/goal dead time (catches AFK
without disconnecting). If either check fails for *any* player, `confidence`
drops to `low_confidence` for **every** report in that lobby. This is now a
permanent, general-purpose fix — it applies to every future rank pull and
every real replay scored through `rleval`, not just this Bronze sample.
Unit-tested (`scoring/src/coverage.rs`) and integration-tested against the
committed sample replay (`scoring/tests/coverage.rs`); full workspace
`cargo test` and `cargo clippy -D warnings` stay clean.

Re-scoring Bronze with the gate live caught **exactly the 7 replays** found by
hand above — automatically, with no manifest edits or hand-curated exclusion
list:

| replay | issue |
|---|---|
| `db693699…` | player left at 22% of the match |
| `2879e495…` | player left at 36% |
| `1f330704…` | player left at 40% |
| `34d03f75…` | player left at 50% |
| `82746fa8…` | player left at 81% |
| `b7daa95e…` | player left at 83% |
| `93bec395…` | two players idle for a combined ~140s mid-match (no disconnect) |

**Every number from here on uses only the 23 clean replays (90 rank-joined
player-observations: 49 Bronze I, 17 Bronze II, 24 Bronze III)** — the 28
observations from the 7 flagged replays are excluded via the same
general-purpose `confidence != ok` filter the codebase already uses elsewhere
(`calibrate.rs`'s "confident only" column), not a bespoke Bronze-only list.

**Caveat that matters throughout:** 90 players spanning only 3 of the ladder's
22 tiers is a *restricted range* — correlations computed **within** Bronze are
naturally weak/noisy (a handful even flip sign vs. the full-corpus direction:
`recovery_speed`, `aerial_presence`, `reverse_driving`, `pace` — this is the
same "inverted" warning the calibration harness prints on any narrow slice,
not a rubric bug; the full corpus already validates these directions at
composite-vs-rank ρ≈0.79–0.81). The reliable signal here isn't "does X
correlate with Bronze I vs III", it's **where Bronze's average sits against
the entire ranked population's calibrated floor and ceiling** — that
comparison is what the rest of this document leans on. Growing Bronze to the
corpus's standard 250 (`expand_manifest.py --bucket bronze --per-tier 250`)
would sharpen the within-bracket estimates.

## Headline: Bronze sits at the floor of the entire rubric

| | value |
|---|---|
| Composite (0–100 scale, corpus-calibrated) | **mean 21.7**, median 20.1, range 4.8–81.3, σ=12.0 |
| First-man discipline | 28.9 |
| Second-man discipline | 36.9 |
| **General fundamentals** | **15.8** |
| Licence band | 70% *Unranked* (below the Silver-Licence bar), 21% Silver, 3% Gold, 2% Platinum, 2% Diamond, 1% Pacifist Master |

General fundamentals — boost economy, car control, positioning discipline,
the metrics that don't depend on a 1st/2nd-man role — is the weakest of the
three sub-scores by a wide margin, and it's also a full third of the
composite. Bronze's ceiling problem is fundamentals, not role execution.

A few Bronze-ranked players still score into Gold/Platinum/Diamond-Licence
territory on this positioning rubric (composite up to 81.3) — a mechanically
limited but positionally disciplined player can out-score their MMR here, and
some of these are likely placement noise. They're the minority; the bracket's
center of mass is squarely at the bottom of the scale.

## The #1 leak, by a landslide

`main_leak = boost_management` for **82 of 90 players (91%)**, routing to the
*"Fundamentals / boost"* coaching chapter. Everyone else splits between
`aerial_presence` (4), `reverse_driving` (3), and `facing_ball_share` (1) — no
other leak comes close. Boost management is also the single highest-weighted
general-fundamentals metric in the composite (w=0.054), so it's simultaneously
the most common **and** the most consequential thing holding Bronze back.

## Where Bronze sits against the whole ranked ladder

Each metric's `floor`/`ceiling` below are the curve anchors from the shipped,
corpus-wide `fitted_config.json` — i.e. calibrated across every rank
Silver→GC. `pos` is where Bronze's mean falls on that 0–100% floor→ceiling
line (negative = literally worse than the worst-calibrated anchor in the
entire ranked population). Rows are oriented "higher pos% = closer to how the
rest of the ladder plays" — for a couple of these the *raw* direction is
counter-intuitive, called out below the table.

| metric | weight | Bronze mean | corpus floor | corpus ceiling | position |
|---|--:|--:|--:|--:|--:|
| **facing_ball_share** | 0.017 | 0.448 | 0.441 | 0.335 | **−6.5%** |
| pace (avg speed, uu/s) | 0.012 | 1151.4 | 1168.9 | 1478.8 | **−5.6%** |
| boost_starvation (mean zero-boost run, s) | 0.002 | 3.57 | 3.49 | 1.52 | **−4.2%** |
| **boost_management** | **0.054** | 0.082 | 0.086 | 0.201 | **−3.8%** |
| ball_chase_index | 0.002 | 0.272 | 0.271 | 0.196 | **−1.7%** |
| overcommit_rate | 0.007 | 0.171 | 0.174 | 0.057 | 3.4% |
| double_commit_rate | 0.019 | 0.054 | 0.056 | 0.015 | 4.9% |
| reverse_driving | 0.036 | 0.107 | 0.113 | 0.030 | 7.4% |
| goalside_discipline_team | 0.000 | 0.680 | 0.651 | 0.858 | 14.0% |
| aerial_presence | 0.042 | 0.073 | 0.054 | 0.168 | 16.2% |
| goalside_discipline_1st | 0.000 | 0.504 | 0.455 | 0.708 | 19.4% |
| first_touch_value | 0.002 | 0.190 | 0.091 | 0.441 | 28.4% |
| transition_readiness | 0.000 | 0.510 | 0.429 | 0.677 | 32.7% |
| challenge_timing | 0.000 | 0.378 | 0.283 | 0.545 | 36.3% |
| central_support_fraction | 0.030 | 0.371 | 0.289 | 0.487 | 41.3% |
| recovery_speed | 0.005 | 0.801 | 1.050 | 0.502 | 45.5% |
| possession_retention | 0.000 | 0.471 | 0.313 | 0.586 | 58.1% |

**`facing_ball_share` reads backwards from intuition, on purpose.** The
config's own comment (`scoring/src/config.rs`) documents why: on the full
723-player corpus, *more* time spent staring at the ball correlates with
*lower* rank (ρ≈−0.47) — ball-watching/tunnel vision is a beginner habit, and
better players spend more time scanning boost/teammates/opponents while
tracking the ball peripherally. So the curve is deliberately "lower is
better," and Bronze's mean (0.448) sits *above* even the worst-calibrated
reference point (0.441) — Bronze fixates on the ball more than nearly anyone
else on the entire ladder. That makes it, in raw floor-to-ceiling terms, the
single most extreme metric on this whole sheet — just not the most
*consequential* one, since its composite weight (0.017) is a third of
`boost_management`'s.

Five metrics sit below the corpus-wide floor outright (negative position);
three more (`overcommit_rate`, `double_commit_rate`, `reverse_driving`) sit
under 8% of the way to the ceiling. Two of those eight —
`boost_management` (w=0.054) and `reverse_driving` (w=0.036) — also carry real
composite weight, which is exactly why they surface as leaks above. By
contrast `possession_retention` and `recovery_speed` are unremarkable — once
Bronze players do get a touch or go airborne, they aren't dramatically worse
than the rest of the ladder at the basic mechanics of it.

Within Bronze itself (I→III, n=49/17/24 — thin per-tier samples, read
qualitatively), a handful of metrics show a mild, consistent climb:
`boost_management` (0.078→0.084→0.089), `boost_starvation` (3.70→3.54→3.33s,
improving), `double_commit_rate` (0.055→0.054→0.051, improving),
`goalside_discipline_team` (0.669→0.680→0.705), and most clearly
`first_touch_value` (0.150→0.191→0.272). The most extreme metrics —
`overcommit_rate`, `ball_chase_index`, `reverse_driving`, `pace` — stay flat
or noisy across all three tiers. Read together: a few fundamentals start
improving even within Bronze, but the headline-bad habits (overcommitting,
ball-chasing, ball-watching) are **Bronze-wide plateaus** that don't visibly
budge until something forces a change (i.e., until Silver).

## What this looks like on the field

**Boost economy** (`bcstats`, all values are per-player match averages):
- Average gauge: 46/100. 19.2% of the match at 0 boost, 15.7% at full — bronze
  swings between empty and capped rather than running a steady mid-range tank.
- 227 boost/min collected (`bpm`), of which ~22% (288 of 1284 total) comes
  from pads in the opponent's half — not necessarily deliberate "stealing",
  but time spent forward-of-the-play picking up pads that a stricter rotation
  would leave for a teammate.

**Movement:**
- 61.8% of the match at "slow" speed (below boost-speed threshold); only 5.5%
  supersonic. Average speed 1151 uu/s sits *below* the corpus-wide floor
  (1169) — Bronze isn't just slower than other ranks, it's slower than the
  10th-percentile player anywhere on the ladder.
- 73.8% ground / 23.0% low-air / 3.2% high-air — Bronze does get off the
  ground somewhat regularly (challenges, small hops), just almost never high.

**Positioning:**
- 49.0% of the match in the defensive third; 69.7% behind the ball overall.
- `most_back` and `most_forward` both average ~50% — i.e., on a 2-man team,
  no player is durably "the back" or "the forward"; roles swap constantly
  rather than settling into a stable 1st/2nd-man shape.
- Facing the ball 48.6% of the time — and per the direction correction above,
  this is Bronze's least-bad-looking positioning stat, not its worst: the
  shipped rubric's own evidence says *less* time locked onto the ball is
  what correlates with rank, and Bronze already ball-watches more than the
  ladder's worst-calibrated reference.
- 1.72 goals conceded, on average, while this player was the last man back —
  a high rate of backline breakdowns (interpret as a rate across replays of
  varying goal totals, not an absolute grade).

**Demos:** 0.63 inflicted / 0.63 taken per player per match — a wash; demos
aren't a differentiator at this level.

## Mechanics: the floor is not zero

The mechanical-skill detector (`replay-skills`) shows Bronze already clears a
real floor of car control — the gap to Silver is decision-making and
consistency, not "learn a new skill from scratch":

| skill | % of players with ≥1 | attempts/player/match |
|---|--:|--:|
| supersonic | 100% | 13.7 |
| kickoff first touch | 78% | 1.6 |
| boost steal | 74% | 1.6 |
| power shot | 68% | 1.6 |
| aerial (a touch above the height floor) | 67% | 2.0 |
| redirect | 63% | 1.5 |
| wall play | 56% | 1.0 |
| flick | 53% | 0.9 |
| ceiling play | 33% | 0.5 |
| demo | 31% | 0.6 |
| double touch | 13% | 0.2 |
| ground dribble | 11% | 0.1 |
| air dribble | 6% | 0.1 |

Everything above "double touch" already happens *sometimes* for most players
— the single-contact mechanics (shots, redirects, wall reads, aerial touches)
are present at a real rate. What's essentially absent is **sustained ball
control**: ground dribble, air dribble, and double touch — the mechanics that
require controlling the ball over more than one contact — sit at 6–13% of
players and around 0.1–0.2 attempts/match. Bronze offense is built entirely
from single, opportunistic contacts; nobody is manufacturing a second touch
yet.

## The limits of Bronze, synthesized

1. **Boost is reactive, not managed.** The highest-weighted fundamental in the
   whole rubric, and Bronze's average is *below* the worst anchor calibrated
   across the entire ranked population. Empty-to-full swings, not a
   maintained mid-range gauge.
2. **Ball-watching, not scanning.** `facing_ball_share` is the single most
   extreme metric on the sheet — Bronze locks its eyes/orientation onto the
   ball more than even the worst-calibrated reference elsewhere on the
   ladder. The shipped rubric's own corpus-wide evidence is that *less* time
   fixated on the ball tracks with *higher* rank (better players scan
   boost/teammates/opponents and track the ball peripherally instead).
3. **No stable rotation shape.** `most_back`/`most_forward` split ~50/50,
   and `ball_chase_index`/`double_commit_rate`/`overcommit_rate` all sit
   under 8% of the way from the ladder's worst anchor to its best — both
   players are frequently drawn to the ball at once, with no habitual "one
   goes, one holds" structure.
4. **Reverse-driving as a turning crutch**, close to the worst end of the
   ladder — backing up to redirect rather than powersliding, despite already
   powersliding ~7×/match, i.e. the tool is used, just not as the default.
5. **No manufactured offense.** All ball-control mechanics that need more
   than one touch (dribbling, air dribbling, double touches) are essentially
   absent; scoring depends entirely on the ball bouncing into a good spot for
   a single-contact shot/redirect.

## How to improve beyond Bronze

In the order the data says they matter most (weight × how far below the
ladder's calibrated range, matching how the rubric itself picks a leak):

1. **Fix boost management before anything else.** It's the single
   highest-weighted fundamental metric *and* one Bronze scores worst on in
   absolute terms. Concretely: never let the tank run to 0 without a plan to
   refill on the way back to position, and don't detour for a pad that takes
   you out of shape — collect the ones already on your rotation path.
2. **Practice looking away from the ball.** Counter-intuitive, but it's the
   single most extreme deviation on the whole sheet, and the shipped rubric's
   corpus-wide evidence backs the direction: build the habit of quick glances
   at boost pads, teammate position, and open net/opponents between touches,
   trusting peripheral vision to track the ball itself.
3. **Stop the double-commit / ball-chase reflex.** Rule of thumb: if a
   teammate is already engaging the ball, the job is to rotate to cover their
   space, not to also go for the ball. This single habit change is what
   starts producing a real 1st-man/2nd-man shape instead of the current
   ~50/50 role blur, and it's also most of the fix for overcommitment (fewer
   50/50s thrown from bad angles once the reflex to chase every ball is gone).
4. **Replace reverse-driving with powerslide turns.** The mechanic is already
   in Bronze's toolkit (~7 powerslides/match) — it just needs to become the
   default way to redirect instead of backing up.
5. **Expect pace and boost-starvation to improve as side effects of 1 and 3.**
   Both sit below the ladder's calibrated floor, but neither carries much
   composite weight on its own — they read as *symptoms* of chasing the ball
   and running the tank dry, not separate problems needing their own fix.
6. **Only after 1–4 are automatic, start layering sustained ball control**
   (ground dribble → air dribble). It's the one mechanical category still at
   ~0 usage, but every other single-contact mechanic (aerial touches, power
   shots, redirects, wall reads) is already present at a "sometimes" rate —
   that side of the game needs more reps and consistency, not a new skill
   from zero.

## Reproducing / extending this analysis

```sh
# grow the Bronze sample toward the corpus's standard size
BC_TOKEN=<token> python assets/corpus/expand_manifest.py --bucket bronze --per-tier 250
BC_TOKEN=<token> python assets/corpus/refresh_corpus_replays.py --bucket bronze
python assets/corpus/refresh_manifest_playlist.py --skip-missing

# score every Bronze replay through the shipped (unchanged) production rubric;
# the lobby-completeness gate (scoring/src/coverage.rs) runs automatically and
# marks a whole lobby low-confidence if any player left/went AFK
./target/release/replay-scoring --config assets/corpus/fitted_config.json --json out.json <replay>
./target/release/replay-analyzer --bc-stats out.json <replay>
./target/release/replay-skills --json out.json <replay>
# then filter aggregation to reports where "confidence": "ok"
```

`calibrate` was deliberately **not** re-run against the full manifest for this
write-up — only 30 Bronze `.replay` files are present locally (the other 996
corpus replays are gitignored and weren't re-downloaded), and running
`calibrate` against a manifest where almost every entry's file is missing
would refit `fitted_config.json`/`rank_norms.json` off a Bronze-only sample,
silently corrupting the existing full-corpus calibration. The within-Bronze
correlations quoted above came from pointing `calibrate` at an isolated,
filtered manifest (Bronze entries only, copied outside `assets/corpus/`) so
its writes never touched the committed artifacts.
