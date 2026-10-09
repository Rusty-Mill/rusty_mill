# Claude Code prompt — Rocket League replay analyzer

> Copy everything below the line into Claude Code. Optionally drop these reference files into
> the repo root first so it can read them: `pacifist-system-teardown.md`,
> `replay-scoring-service-spec.md`, `canonical-match.json` (a sample output). The prompt works
> without them, but they add context.

---

You are setting up and continuing a **Rocket League replay analyzer** — the foundational data
layer for a future "decision-discipline" scoring engine. Work in a new repo (or the current dir).

## What this is and why

The end goal is a service that scores a 2v2 replay on positional/rotational decision-making.
But the **analyzer must be built and validated first, as a standalone, scoring-agnostic
component.** It takes a `.replay` and emits a neutral **canonical match model** (uniform per-frame
world state). Scoring is just one future consumer of that model — keep the seam clean. Do not put
any scoring/"Pacifist" logic in the analyzer.

A working first cut already exists and is validated; your job is to (a) re-establish it as a
properly modularized project with tests, then (b) implement the next milestone. If
`replay-scoring-service-spec.md` is present, treat it as the downstream design the canonical model
must serve.

## Stack & constraints

- **Language:** Rust. Decode layer = the `boxcars` crate. (A Python port can come later via a
  thin worker; not now.)
- **boxcars version:** run `rustc --version` first. If Rust ≥ 1.80, use the latest `boxcars`
  (`0.11+`). If older, pin `boxcars = "=0.9.13"` (0.11.x uses `split_at_checked`/`split_first_chunk`
  which need ≥1.80). The network-frame API used below is stable across both.
- **Style:** SOLID / DRY / KISS; composition over inheritance; ports-and-adapters at the
  parse boundary so the decoder is swappable; explicit over implicit; small modules, separation
  of concerns; rustdoc on public items; robust error handling (no `unwrap` on replay data paths —
  return `Result`). **Tests are required**, including golden-file tests on the pure analyze layer.
- **Validation-first mindset:** a number you can't validate is a bug you haven't found yet.

## Current state (the validated first cut — reproduce this, then improve it)

A ~200-line single-file analyzer already:
1. Decodes with `ParserBuilder::new(&data).on_error_check_crc().must_parse_network_data().parse()`.
2. Pulls header props: `MapName`, `TeamSize`, `RecordFPS`, `Team0Score`/`Team1Score`, and the
   `PlayerStats` array (Name/Team/Score/Goals/Assists/Saves/Shots).
3. Walks `network_frames`, classifies new actors as ball/car by archetype-name substring, binds
   car→PRI→name (`Engine.Pawn:PlayerReplicationInfo` → `Engine.PlayerReplicationInfo:PlayerName`),
   reads rigid bodies (`TAGame.RBActor_TA:ReplicatedRBState` → `Attribute::RigidBody`), and
   reconstructs per-frame world state by **carry-forward** of last-known positions/velocities.
4. Emits a canonical JSON: `{ replay_id, parser_version, map, team_size, record_fps, num_frames,
   duration_s, team_scores, players[], frames[] }` where each frame is
   `{ t, ball: {x,y,z}|null, cars: [{ actor_id, player, p:{x,y,z}, v:{x,y,z} }] }`.

**Validation it passed** (replicate these as assertions): ball position bounds fall within the real
arena (x ≈ ±4096, y ≈ ±5120, z 0–2044); a kickoff frame shows the ball at (0,0,~93) with cars on
symmetric spawn positions. Both confirm correct decode + reconstruction.

**Known issues to fix as you port it:**
- `parser_version` currently prints the local crate version, not the boxcars version. Emit the real
  boxcars version (or both).
- It's a single `main.rs`. Refactor into modules (see structure below).

## Target project structure

```
src/
  main.rs            # CLI: parse args, call lib, print summary / write JSON
  lib.rs             # re-exports
  decode/
    mod.rs           # ReplayParser trait (port)
    boxcars_adapter.rs
  model.rs           # canonical types (serde): CanonicalMatch, FrameOut, CarState, Vec3, PlayerMeta, PlayerTrack
  analyze/
    mod.rs
    reconstruct.rs   # carry-forward world-state reconstruction
    identity.rs      # actor→PRI→player binding, recycled-actor coalescing
    resample.rs      # fixed-Hz grid
    normalize.rs     # coordinate normalization (each team attacks +Y)
    features.rs      # derived per-frame/per-player features
  field.rs           # arena constants
tests/
  golden.rs          # canonical-model golden files from sample replays
  fixtures/          # hand-built CanonicalMatch fixtures isolating one behavior each
assets/replays/      # sample .replay files (see below)
```

## Tasks, in order

**T0 — Scaffold + reproduce.** Set up the project, add deps (`boxcars`, `serde`, `serde_json`),
get sample replays (download the boxcars repo tarball from
`https://codeload.github.com/nickbabcock/boxcars/tar.gz/refs/heads/master` and copy a few
`assets/replays/good/*.replay`; the unauthenticated GitHub *API* rate-limits, the tarball doesn't).
Port the first cut into the module structure above. Add a `--json <path>` flag and a stderr summary.
Fix the `parser_version` wart.

**T1 — Per-player track coalescing (the real next problem).** RL recycles car actor IDs on every
respawn, demo, and goal reset, so one player currently fragments into dozens of actor-ID segments.
Coalesce all car-actor segments belonging to the same player into **one continuous `PlayerTrack`**,
keyed on the **stable PRI/player identity** (not the car actor id). Mark respawn/dead gaps explicitly
(don't carry-forward across a gap). Output adds `players[].track` or a parallel `tracks[]` keyed by
player.

**T2 — Resample + normalize.** Resample tracks to a fixed 30 Hz grid by interpolation (every frame
has all live actors). Then normalize coordinates so **each team attacks +Y**, producing a
per-player "attacking-direction" frame so left/right and own/opp half are comparable across teams.

**T3 — Boost extraction.** Link `TAGame.CarComponent_Boost_TA:ReplicatedBoostAmount` (byte 0–255,
≈ value/2.55 for %) to its car via the boost component's `Vehicle` link
(`TAGame.CarComponent_TA:Vehicle` → car actor). Add `boost` to per-frame car state.

**T4 — Events.** Derive touches (ball-velocity discontinuity + nearest car), possessions
(touch chains by team), kickoffs (ball-centered + countdown), demos (the demolish attributes), goals.

**T5 — First features + external validation.** Compute a few derived aggregates (boost used, time
supersonic, touch counts, possession time, mean distance-to-ball) and **cross-check them against
ballchasing.com (API) or carball for the same replay**, asserting agreement within tolerance. If
they disagree, the reconstruction is wrong — fix that before trusting anything else.

## Definition of done for this session
T0 and T1 complete: a modular project that builds, runs on a sample 2v2, emits the canonical model
with **coalesced per-player tracks**, passes the arena-bounds + kickoff-geometry validations as
automated tests, and has at least one golden-file test and one fixture-based unit test for coalescing
(assert a known recycled-actor case collapses to the right number of player tracks).

## Technical reference (boxcars)

- Parse: `ParserBuilder::new(&data[..]).on_error_check_crc().must_parse_network_data().parse()?`
- Resolve object name → id: `replay.objects.iter().position(|o| o == name)` → `ObjectId(i as i32)`.
- Per frame: `new_actors` (actor_id, object_id), `updated_actors` (actor_id, object_id, attribute),
  `deleted_actors`.
- Object strings: rigid body `TAGame.RBActor_TA:ReplicatedRBState`; car→PRI
  `Engine.Pawn:PlayerReplicationInfo`; PRI→name `Engine.PlayerReplicationInfo:PlayerName`;
  team `Engine.PlayerReplicationInfo:Team`; boost `TAGame.CarComponent_Boost_TA:ReplicatedBoostAmount`;
  boost→car `TAGame.CarComponent_TA:Vehicle`. Ball/car archetypes contain substrings `Ball`/`Car`.
- `Attribute::RigidBody(RigidBody { location: Vector3f, rotation, linear_velocity: Option<Vector3f>, .. })`.
  `Attribute::ActiveActor(ActiveActor { actor, .. })` for the PRI/Vehicle links;
  `Attribute::String(name)` for names. `Vector3f` is already in uu (decoded as raw/100).
- Field constants (`field.rs`): side wall x ≈ ±4096, back wall y ≈ ±5120, ceiling z ≈ 2044,
  goal mouth on ±Y. Use these in validations.
- See the `examples/demo.rs` in the boxcars repo for the canonical car→PRI→name binding pattern,
  and note its warning about actor-id recycling — which is exactly what T1 solves.

Start with T0, show me the project tree and the summary output on a sample replay, then proceed to T1.
