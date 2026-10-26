# ADR-0085: Show sim runs in the existing replay viewer

- Status: Accepted
- Date: 2026-10-24
- Deciders: project owner (board card w5, 2026-10-24); design by Claude
- Related: board cards r4, b5, b6, b7; ADR-0005 (capture format); `rleval/docs/replay-viewer.md`; RB-VERIFY-003
- Supersedes/Superseded by: none

## Context

Debugging the port means reading numbers: `rb-verify --self`, `--self-trace` and
`--self-onestep` print where a simulated car leaves a recording, but not what the two
trajectories look like. A dodge that rotates the wrong way or a wall curve that sheds
speed is obvious on screen and slow to see in a table.

The repo already has a 3D viewer. `replay-viewer` turns a `CanonicalMatch` into a `Scene`
(a time grid of `SceneFrame { t, ball, cars: [SceneCar { pri, p, rot, boost }] }` plus
players, events and field) and renders it to one self-contained three.js HTML file, with
play, pause, scrub and an orbit camera. three.js is vendored for offline files and a
headless-GL smoke test (`tests/gl_smoke.mjs`) runs in CI.

On the physics side, every source we compare already produces `rb_domain::PhysicsFrame`
lists: BakkesMod captures and tape-bot fixtures (ADR-0005 JSONL, read by
`rb_capture_ingest`) and the port's own candidate runs (`simulate_recorded`).

## Decision drivers

- Reuse what works: one renderer, one smoke test, one HTML format.
- Keep the sim core free of viewing code (no rendering or serde in `rb_physics_bullet`).
- Compare two trajectories at once: the recording and the port, on the same timeline.
- Small slices, each useful on its own.

## Considered options

1. **Extend `replay-viewer` with a `PhysicsFrame` input** (chosen).
2. A native window in the sim (e.g. a Rust 3D crate): new dependencies, a second renderer,
   and drawing code next to the physics.
3. A live socket server streaming sim state to a browser: needed only for interactive
   runs, which nothing asks for yet; recorded playback covers today's debugging.

## Decision

Add a `physics` module to `replay-viewer` that maps `&[PhysicsFrame]` to the existing
`Scene`. The input is files, not code: the viewer reads ADR-0005 JSONL through
`rb_capture_ingest` (whose only dependencies are `rb_domain`, `serde` and `serde_json`),
so it never links the physics. The recording and an optional candidate are two files:

```
replay-viewer capture.jsonl [--sim candidate.jsonl] [--html out.html] [--offline]
```

- `rb-verify` gains a way to write its candidate run as ADR-0005 JSONL (for example
  `--self <capture> --write-candidate out.jsonl`), so the sim side stays in the
  composition root that already owns `simulate_recorded`.
- Each `PhysicsFrame` car becomes a `SceneCar`: position as is, the quaternion converted to
  the viewer's `[pitch, yaw, roll]`, boost from `boost_amount`. Sim cars get their own
  `pri` range and a `ghost` flag on `ScenePlayer` (a new optional field, absent for
  replays), drawn translucent over the recorded car.
- Frames stay at the capture rate (120 Hz) rather than the replay path's 30 Hz grid. The
  viewer already plays by `t`, so only file size grows: about 1 MB per minute per car.
- Positions in `Scene` are rounded to 1 uu, as for replays. Error numbers shown in the
  viewer come from `rb_domain::divergence`, computed before rounding.
- Replay-only fields (events, stats, win probability, roles) stay empty; the replay path
  is unchanged.

Slices, one PR each:

- **b5, render one run:** the `physics` adapter, the CLI input, `--write-candidate`, the
  ghost car. Tests: a golden `Scene` from a short fixture, and the GL smoke test on a
  capture-built page.
- **b6, vectors and inputs:** optional per-car velocity on `SceneCar` drawn as an arrow;
  an input timeline strip (throttle, steer, pitch, yaw, roll, jump, boost, handbrake) from
  `CarState.input`, recorded beside simulated. Velocity and input are new optional fields,
  so replay scenes do not change.
- **b7, contacts and the real mesh:** wheel and ball contact points need a read-only debug
  record out of `rb_physics_bullet`. It is written by an adapter outside the core (a probe
  trait or a returned struct, decided in that PR) and saved as a separate JSONL beside the
  candidate. The arena box is replaced by the collision mesh the sim uses. A live socket
  feed is considered only after recorded playback has been in use.

What stays out of the sim core: rendering, serde, file output, and any viewer type. The
core exposes state it already has; adapters turn it into files.

## Consequences

### Positive

- One viewer for replays, captures and sim runs; the smoke test covers all three.
- Seeing the port beside the recording turns `rb-verify` findings into something a person
  can check in seconds.
- `replay-viewer` gains only `rb_capture_ingest`, a small crate in the same layer.

### Negative / tradeoffs

- `Scene` grows three optional fields (`ghost`, velocity, input). They are skipped when
  absent, but the type now serves two kinds of source.
- 120 Hz pages are larger than replay pages; a 90 s raw clip with a ghost is about 3 MB
  before three.js.
- No live view until b7 is done and a need appears.

## Validation and revisit triggers

- b5 is done when `replay-viewer human/clean-dodge.capture.jsonl --sim <candidate>`
  shows both cars on one timeline and the smoke test passes on that page.
- Revisit the shared `Scene` if the physics fields start to need changes the replay path
  does not want; then split a `SimScene` that the same renderer reads.
- Revisit the live feed if interactive tuning (changing a constant and watching the
  result) becomes a regular task.
