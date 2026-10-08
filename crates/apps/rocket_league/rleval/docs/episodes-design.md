# Episodes — design proposal (assessment rec. #6)

Status: **A (Recovery), B (Challenge) and C (Loss) shipped; D measured and dropped.** Source: `docs/competitor/spire/ASSESSMENT.md` §5 #6.

## Problem
We score recovery, possession loss and challenge only as **per-player aggregates**
(`scoring/src/metrics.rs`). A player can't see *which* moments made the number, and
we can't compare counts with another system. Every claim should link to a moment.

## Goal
One small `Episode` type; three emitters (recovery, loss, challenge); each metric
becomes a reducer over its episodes, so **the list and the score cannot drift**.
Serve the rows and let a click seek the 3D viewer to the moment.

## Future growth
Not in v1 — each needs a definition we can't validate yet (assessment §2.2, §6.2) —
but the design leaves room: a new kind is one enum variant plus one emitter; a new
field on a kind is additive.

| Later | Extends | Needs first |
|---|---|---|
| Forced vs avoidable losses | `Loss` (already carries `danger`, `opp_dist`) | validation against Spire's loss rows |
| Challenge outcomes (won / delayed / lost) | `Challenge` | a per-contest outcome rule; `miss` bits stay |
| Last-man decisions, necessity | new `Decision` kind | a last-man definition (roles are 1st/2nd man today) |
| Approach events (controlled / rushed / hesitated) | new `Approach` kind | thresholds validated on the corpus |
| xG | `Shot` (shipped in assessment #3: strike time, speed, goal-plane `aim`, outcome) | a labelled dataset — the shot log now supplies it |
| ~~Possession chains~~ | shipped (assessment #8): `Chain`, unit = one team's touches ≤3 s apart (`scoring/src/chains.rs`) | — |
| Any tunable threshold | `ScoreConfig` + a version bump | evidence the default is wrong |

## Design

**Type** — an enum, so a field can't exist on the wrong kind (`scoring/src/episodes.rs`):

```rust
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Episode {
    Recovery  { pri: i32, t0: f32, dur: f32, done: bool },
    Loss      { pri: i32, t: f32, at: [f32; 2], danger: f32, opp_dist: f32 },
    Challenge { pri: i32, opp: i32, t0: f32, dur: f32, miss: u8 }, // miss bits: BOOST|FACE|LATE
}
impl Episode { pub fn t(&self) -> f32; pub fn pri(&self) -> i32; }
```

**Definitions are today's, moved not changed** (`metrics.rs`):
- *Recovery*: airborne end → wheels-down, upright, facing ball (`facing_cos_min`); `dur` is
  capped at `recovery_cap_s`; `done` = recovered within the cap (`recovery_speed`, :553).
- *Loss*: a touch whose **next known touch is the other team**; `danger` = depth in own
  half (the existing weight), `at` = ball xy, `opp_dist` = nearest opponent to the ball
  (new evidence, additive) (`dangerous_turnover`, :288).
- *Challenge*: a 1st-man ↔ opponent-1st-man contest within `challenge_radius_uu`; `miss`
  bits = failed boost / facing / late tests, `ok` = `miss == 0` (`challenge_timing`, :420).

**Single source of truth** — emitters are `pub(crate)` and *hold the scan loops*; the
metric functions shrink to reducers:

| Metric | Reducer over episodes |
|---|---|
| `recovery_speed` | mean `dur` of `Recovery` |
| `challenge_timing` | share of `Challenge` with `miss == 0` |
| `dangerous_turnover` | Σ `danger` of `Loss` ÷ the player's touches |

`possession_retention` keeps its own touch loop (its numerator counts *unknown-team* next
touches as not kept; a loss does not — different set, so not forced into one).
Store `dur`, not `t1`: `(t0 + d) − t0 ≠ d` in f32, and aggregates must stay **bit-identical**.

**Entry point** — one lobby-wide call, not per player:

```rust
pub fn extract(m: &CanonicalMatch, cfg: &ScoreConfig) -> Vec<Episode>  // sorted by t
```
It builds frames + roles **once** (`score()` rebuilds them per player today — 6× a lobby;
see PR-D). `Report` is untouched, so **no golden file changes**.

**Delivery**
- `app/src/pipeline.rs`: `Analysis.episodes` (measured on `419a`: 100 recoveries ≈ 8 KB).
- `viewer/src/render.rs`: `window.seek = seek;` (its script is a module, so `seek` isn't
  reachable today). Same-origin `srcdoc` iframe ⇒ `contentWindow.seek(t)` works.
- `app/src/ui.rs`: one **Moments** tab — player filter, kind chips, table (time, player,
  detail); row click → open the 3D Viewer tab and seek. The table *is* the raw-rows view.

```
frames+roles ─► emitters ─► Vec<Episode> ─┬─► metrics (reducers) ─► Report (unchanged)
   (once)                                  └─► Analysis.episodes ─► Moments tab ─► viewer.seek(t)
```

## Tests
1. **Drift guard (permanent):** for each player on both sample replays, reducer over
   `extract` == the `Report` raw metric, **bit-equal**.
2. **No behaviour change:** every existing scoring/calibrate/golden test passes unmodified.
3. Per-emitter unit tests on synthetic frames (reuse `scoring/tests` builders): done vs
   timed-out recovery; each `miss` bit; loss ignores unknown-team next touch.
4. Pin the `419a` counts **after the first run** (recorded, not guessed).
5. Headless-browser smoke (existing GL-smoke pattern): row click moves the viewer timeline.

## Validation against Spire
Assessment §6.2 items 2–4 once its raw rows are available: align by timestamp (±0.5 s),
report count and per-player agreement. **Differences are expected** (§2.2): ours is
touch-based and needs facing to call a recovery done. The list makes them inspectable.

## Staging — four small PRs, ≈ 8 dev-days
| PR | Scope | Days |
|---|---|---|
| A | `Episode`, `extract`, **Recovery only**, reducer + drift test, `Analysis.episodes`, Moments tab, viewer seek — a full vertical slice | 3 |
| B | Challenge emitter + reducer | 2 |
| C | Loss emitter + `opp_dist` + reducer | 2 |
| D | ~~Build frames/roles once per lobby in `score_all`~~ — measured, dropped: `score_all` is 33 ms on `419a` and sharing frames saves ≈3 ms (<1% of the 0.45 s analysis); `build_frames` is not the cost | 1 |

A proves the plumbing end to end; B and C are then pure additions.

## Budgets (fail the PR if exceeded)
- Analysis wall time on `419a` ≤ **+5%** over the 0.47 s baseline (release).
- `Analysis` JSON ≤ **+50 KB** (gzip ≤ +10 KB).
- Net diff per PR ≤ ~250 lines excluding tests; no new dependency.

## Risks and decisions
- **Loss ignores dead-ball / goal-ended possessions** (needs a next touch). Recommend
  keeping it: it matches the existing metric; revisit with Spire's rows (§6.2 #2).
- **"Challenge" is a 1st-man 50/50, not a last-man decision.** Naming it honestly beats
  implying parity; last-man logic is a separate, later change.
- **Refactor risk** is confined by the bit-equal drift test.
- **Stay terse:** if an emitter needs a config knob, that is a smell — stop and reassess.

## Decisions
1. **Standalone Moments tab** — one panel, no coupling to the Improve tab.
2. **Four-PR cut (A–D)** as staged above, one PR at a time, A first.
3. Future-growth list stands as written; revisit per item when its prerequisite lands.
