# Pacifist Score — Analyzer Design

A foundation doc for the replay-analysis tool. It moves top-down: concept model →
score rubric → replay-data mapping → Rust architecture → first slice. Each layer
defines the requirements for the one beneath it.

> **Concept sourcing note.** The system concepts below are framed in general Rocket
> League terms so they can drive a scoring model. The *exact* Pacifist definitions
> (thresholds, what precisely counts as a "good" challenge, etc.) are left as
> `TODO(you)` placeholders — fill those from your own understanding of the book.
> The book text itself is not reproduced here.

---

## 1. Concept model — the system as a state machine

The score only means something if the code mirrors how the system actually carves
up the game. Four moving parts:

**1.1 The situation quadrant.** Two binary axes give the four main situations:

- *Possession*: do we control the ball, or do they? (`Ours` / `Theirs` / `Neutral`)
- *Territory*: is play in our defensive half or the attacking half?

That yields Offensive-Offence, Offensive-Defence, Defensive-Defence,
Defensive-Offence. Model possession as three-valued (neutral is real and common),
even though the book frames it as two — a `Contested`/`Neutral` state keeps you from
forcing every frame into a possession bucket it doesn't belong in.

**1.2 Roles.** At any moment each of the two players is 1st man (the one engaging /
applying pressure / nearest the play) or 2nd man (cover, shadow, net). Role is a
*derived, per-moment* property, not a fixed assignment — it flips constantly. Getting
role attribution right is the linchpin of the whole analyzer; most metrics are
"did the 1st/2nd man do the right thing *for their role* in *this situation*."

**1.3 Principles (the things the score rewards).** In Pacifist terms these are about
control and patience rather than mechanical flash:

- **Funnel discipline** — forcing the opponent toward low-percentage areas instead
  of letting them through the middle.
- **Shadow quality** — 2nd man holding a covering position/distance rather than
  committing.
- **Challenge timing** — 1st man engaging at the right moment; not early, not late,
  not at all when shadowing is correct.
- **Commitment discipline** — avoiding the double-commit (both players up, net open).
- **Rotation soundness** — getting back/out of the way, keeping a man back.
- **Shot selection** — taking high-percentage opportunities over hero gambles.
- **Boost economy** — not arriving at the key moment empty.

`TODO(you)`: pin the precise definition + threshold for each from the book.

**1.4 The ethos.** "Pacifist" = patience and control. The score should *reward
restraint* and *penalise over-extension*, which is the opposite polarity from a
raw-aggression or highlight-reel metric. Worth stating explicitly because it drives
sign conventions later (e.g. a well-held shadow scores higher than a flashy whiffed
challenge).

---

## 2. The score rubric

### 2.1 Shape

A `PacifistScore` is a weighted aggregate of independent **dimensions**, each scored
0–100, each with a **confidence** (how reliably we could measure it from this replay).
Per-player, per-match; roll up to per-game and per-segment later.

```
score = Σ (weight_d · dimension_d · confidence_d) / Σ (weight_d · confidence_d)
```

Confidence-weighting matters: some dimensions are clean to measure (boost on hand),
others are proxy-heavy (challenge timing). Low-confidence dimensions shouldn't swing
the headline number as hard as clean ones. Weights live in config, never hard-coded.

### 2.2 Dimensions

| Dimension | What it captures | Primary signal | Confidence |
|---|---|---|---|
| Commitment discipline | Avoiding double-commits / man-back kept | Both players' positions vs ball + own goal | High |
| Positioning fit | Right area for situation × role | Player pos vs situation-appropriate zone | Med |
| Rotation soundness | Back-post / out-of-the-way / recovery to cover | Movement after a touch or challenge | Med |
| Challenge timing | 1st man engages at the right beat | Time-to-ball vs opponent, closing speed at contact | Low–Med |
| Shadow quality | 2nd man holds covering line/distance | 2nd man pos relative to ball→goal vector | Med |
| Shot selection | High-percentage over low | Shot angle/distance, possession before shot | Low |
| Boost economy | Boost on hand at decisive moments | Boost level around touches/challenges | High |
| Over-extension penalty | Punish ball-chasing / no-cover dives | Frequency of "1st man with no 2nd man behind" | High |

Start with the **High**-confidence ones — they're cheap and they already express the
core ethos (discipline + boost + not over-extending). Add the proxy-heavy ones once
the pipeline is proven and you can calibrate them.

`TODO(you)`: assign starting weights. Suggest deriving them by scoring a handful of
replays you'd label "very Pacifist" vs "very not" and tuning until the ranking matches
your judgement — i.e. calibrate against ground truth rather than guessing weights.

### 2.3 Honesty about measurability

Some of these are genuinely hard from replay data alone (intent isn't in the bytes).
The confidence field is how you stay honest about that instead of pretending a proxy
is the real thing. Where a dimension is weak, surface it as "low confidence" in the
output rather than burying it in the aggregate.

---

## 3. From boxcars to a normalized world state

This is the ports-and-adapters seam: `boxcars` is an *adapter*; the domain never sees
a `boxcars::Frame`. The adapter's job is to turn the replay into a clean `Timeline`.

### 3.1 What boxcars gives you

- **Header** — `replay.properties`: map, team sizes, goals, per-player summary stats.
  Cheap, useful for metadata and sanity checks.
- **Network frames** — `replay.network_frames`: the real data. A `Vec<Frame>`, each
  with `time`, `delta`, and actor `new_/updated_/deleted_` lists.
- **Lookup tables** — `replay.objects: Vec<String>` (object_id → class/property path,
  e.g. the ball archetype, the car archetype, the rigid-body state property, the PRI
  property) and `replay.names`. You resolve *what an actor/attribute is* by indexing
  these tables — that's the key move.

> Verify exact `Attribute` variant names and object-path strings against your pinned
> boxcars version's docs and the `objects` table at runtime — they shift between
> versions and I don't want you coding against a name I half-remember. The physics
> live in the rigid-body attribute (`location`, `rotation`, `linear_velocity`,
> `angular_velocity`); boost and demolish come through their own attributes.

### 3.2 The actor-resolution problem

Network frames are a stream of actor spawns/updates/deletes keyed by `actor_id`. To
get a usable world state you maintain a registry across frames:

1. On `new_actors`, record `actor_id → object_id` and classify via
   `replay.objects[object_id]`: ball archetype, car archetype, PRI, boost pickup, etc.
2. On `updated_actors`, apply the attribute to the tracked actor (rigid body → pose,
   boost attr → boost, etc.).
3. Resolve **car → player → team**: cars link to a PRI actor; the PRI carries
   team + name. This mapping is the fiddly bit and deserves its own well-tested unit.
4. On `deleted_actors`, retire the actor (demolitions, goal resets, etc.).

### 3.3 Output: the `Timeline`

Resample the irregular frame stream onto a fixed tick (start ~30 Hz) and emit a
sequence of immutable snapshots:

```rust
pub struct WorldState {
    pub t: Seconds,
    pub ball: BallState,
    pub players: Vec<PlayerState>, // 2v2 → 4, but keep it general
}

pub struct PlayerState {
    pub player: PlayerId,
    pub team: Team,
    pub pose: Pose,          // position + orientation
    pub velocity: Vec3,
    pub boost: u8,           // 0..=100
    pub on_ground: bool,
    pub demolished: bool,
}
```

Everything downstream consumes `&[WorldState]` and never touches boxcars. That keeps
the domain testable with synthetic timelines and lets you swap the parser later
without touching a single metric.

---

## 4. Rust architecture

### 4.1 Crates (modular monolith; split only if a real forcing function shows up)

- **`pacifist-core`** — pure domain. World model, situation/role derivation, metric
  extractors, scoring. No I/O, no boxcars. This is where the tests and the value live.
- **`pacifist-replay`** — the boxcars adapter. Depends on `boxcars` + `pacifist-core`;
  produces a `Timeline`. The only crate that knows boxcars exists.
- **`pacifist-cli`** (or a thin service crate later) — wiring: take a path, parse,
  analyze, render output. Config lives here and is *passed in*, not reached for.

### 4.2 The pipeline

```
.replay ─▶ [pacifist-replay] ─▶ Timeline
                                   │
            [derive] context: per-frame Situation + Role + possession + distances
                                   │
            [extract] each MetricExtractor reads the enriched timeline → DimensionScore
                                   │
            [score] weighted aggregate (config-driven) → PacifistScore
```

### 4.3 Key abstractions

Composition over inheritance: every rubric dimension is an extractor implementing one
small trait, and the analyzer holds a `Vec<Box<dyn MetricExtractor>>`. Adding a
dimension = adding a type + registering it; nothing else changes.

```rust
/// One scoring dimension. Reads the enriched timeline, emits a 0..=100 score
/// with a confidence for one player.
pub trait MetricExtractor {
    fn id(&self) -> DimensionId;
    fn extract(&self, ctx: &MatchContext, player: PlayerId) -> DimensionScore;
}

pub struct DimensionScore {
    pub dimension: DimensionId,
    pub value: Score,        // newtype, clamped 0..=100
    pub confidence: Confidence,
    pub evidence: Vec<Evidence>, // frames/timestamps that drove it — for the UI later
}
```

`MatchContext` is the enriched timeline: the `Vec<WorldState>` plus the per-frame
derived facts (each player's `Situation`, `Role`, possession, key distances) computed
once and shared, so extractors don't each recompute role attribution.

Make illegal states unrepresentable: `Situation`, `Role`, `Team`, `Possession` are
enums; `Score`/`Confidence`/`PlayerId` are newtypes, not loose `f32`/`u32`. Weights
and thresholds are a `ScoringConfig` struct constructed at the edge and threaded down.

### 4.4 Conventions (your house style)

- `Result` + `?` throughout; a `thiserror` error enum per crate; no `unwrap`/`expect`
  outside tests.
- Async: **none here.** This is CPU-bound single-file parsing — keep it synchronous.
  Parallelism, if ever needed, is "analyze N replays across a thread pool," not
  async-ifying the core.
- Tests: unit tests per extractor against hand-built synthetic timelines (cheap and
  precise), plus a golden-replay snapshot test over the whole pipeline. Treat the
  actor-resolution unit as the highest-value test target — most bugs will hide there.
- New deps beyond `boxcars` + `thiserror` (+ `serde` for output): justify before
  adding.

---

## 5. Open decisions for you

- **Role attribution algorithm.** The whole thing rides on this. Simplest start:
  1st man = the teammate with lower time-to-ball; everyone else is 2nd man. Good
  enough to bootstrap, refine later. What's *your* definition?
- **Possession detection.** Last-touch + a decay window? Contested if both touched
  recently? Needs a concrete rule.
- **Segmentation.** Whole-match score only, or per-kickoff / per-possession segments?
  Segments make the eventual coaching output far more actionable.
- **Tick rate.** 30 Hz is a fine default; physics replays are ~30 Hz of keyframes
  anyway. Higher costs memory for little gain.
- **2v2 only, or generalize?** The model above stays general, but you can hard-assume
  2 players/team in the first cut and revisit.

---

## 6. First vertical slice (build this before anything else)

Resist building all eight extractors. Prove the pipe end-to-end with the thinnest
path that touches every layer:

1. `pacifist-replay`: parse one real `.replay` → `Timeline` with ball + 4 cars +
   team mapping. (Hardest part; do it first, test it hard.)
2. `pacifist-core`: derive possession (last-touch) and one situation axis.
3. One extractor — **Over-extension penalty** — it's High-confidence, needs only
   positions, and directly expresses the ethos: count frames where a player is the
   clear 1st man with no teammate between ball and own goal.
4. `pacifist-cli`: print that one number for one replay.

When that runs against a real replay and the number moves in the direction you'd
expect on a chase-heavy vs a disciplined game, the architecture is validated and the
remaining dimensions are just more `MetricExtractor` impls.

---

## Appendix — suggested module tree

```
pacifist/
├── Cargo.toml                 # workspace
├── crates/
│   ├── pacifist-core/
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── world.rs        # WorldState, PlayerState, BallState, Pose
│   │   │   ├── context.rs      # MatchContext, Situation/Role/Possession derivation
│   │   │   ├── metrics/
│   │   │   │   ├── mod.rs      # MetricExtractor trait, DimensionScore, registry
│   │   │   │   └── overextension.rs
│   │   │   ├── scoring.rs      # ScoringConfig, weighted aggregate, PacifistScore
│   │   │   └── error.rs
│   │   └── tests/
│   ├── pacifist-replay/
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── registry.rs     # actor tracking + classification
│   │   │   ├── resolve.rs      # car → player → team
│   │   │   └── timeline.rs     # resample → Timeline
│   │   └── tests/
│   └── pacifist-cli/
│       └── src/main.rs
```
