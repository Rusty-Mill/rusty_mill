# 3D Replay Viewer — `replay-viewer`

> Play a reconstructed match back in 3D in the browser: ball + cars animated over
> the resampled grid, team-coloured with name labels and boost gauges, a
> scrubable timeline, orbit camera, and a live event/skill ticker.

Like `scoring`, `skills`, and `value`, this is a **pure consumer** of the
canonical match model — no parsing, no I/O in the core. It splits cleanly into a
tested Rust core and a presentation layer:

- **`scene.rs`** — `build_scene(&CanonicalMatch, &[SkillInstance]) -> Scene`
  distills the **resampled grid** (uniform 30 Hz, every live actor present,
  already gap-aware) plus match events and detected skills into a compact
  `Scene`. Positions round to 1 uu, rotations to 1e-3 rad, boost to a percent, so
  the embedded payload stays small. Pure and golden-tested.
- **`render.rs`** — `html(&Scene) -> String` embeds the scene into a self-contained
  three.js viewer, mirroring `replay_scoring::render::html`. The output is one
  openable `.html` file.

## Output is one file

```sh
cargo run -p replay-viewer -- assets/replays/42f2.replay            # -> 42f2.html
cargo run -p replay-viewer -- assets/replays/419a.replay --html out.html
cargo run -p replay-viewer -- assets/replays/42f2.replay --json scene.json --no-skills
```

The match data is fully inline; **three.js is loaded from a CDN** (an import map),
so *viewing* needs network access for that one dependency. Vendoring three.js for
a fully-offline artifact is a possible follow-up.

## Controls

Spacebar play/pause · drag the timeline to scrub · ◀/▶ jump ±1 s · speed select
(0.25–4×) · mouse drag orbits, scroll zooms, right-drag pans. The HUD shows the
scoreboard, clock, per-player boost, and the last few timeline events (goals,
demos, kickoffs, touches, and detected skills).

## Coordinates

The scene stays in Rocket League unreal units (Z-up: X side, Y length, Z height);
the web layer sets `DEFAULT_UP = (0,0,1)` so the camera and field read naturally.
Cars are oriented from `[pitch, yaw, roll]` (yaw about Z, then pitch, then roll),
with a white nose marking forward (+X) — exact orientation is a known area to
refine against footage.

## What's tested vs. not

The Rust scene projection and HTML substitution are unit- and golden-tested
(`viewer/tests/`), and the embedded JS is syntax-checked. The **visual** 3D
rendering itself is only verifiable in a browser — there's no headless-GL test in
CI — so treat the rendering layer as the iterate-in-browser part.

## Follow-ups

- Vendor three.js for a fully offline file.
- Tune car orientation against footage; add a chase/ball cam.
- Overlay scoring (roles, 1st/2nd man, leak) alongside the skill ticker.
- Optional playback downsample (`--hz`) to shrink the embedded payload.
