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

The match data is fully inline; by default **three.js loads from a CDN** (an
import map), so viewing needs network for that one dependency. Pass `--offline`
to embed three.js + OrbitControls as `data:` URLs instead — a larger but fully
self-contained file that renders with no network (this is what the CI smoke test
uses). `--hz <rate>` thins the playback grid to shrink the payload.

## Controls

Spacebar play/pause · drag the timeline to scrub · ◀/▶ ±1 s · `n`/`p` next/prev
goal · `k`/`j` next/prev kickoff · speed (0.25–4×) · loop · mouse drag orbits,
scroll zooms, right-drag pans.

**Camera:** `overview` / `goal` / `ball-cam` presets, or click a player row to
lock the camera onto that car (`0` returns to free overview). Goal/demo/kickoff
markers on the timeline are clickable, as is each ticker entry — both seek.

## Overlays

- **Scoring roles** — the 1st man on each team wears a gold ground-ring and a
  `1ST` HUD tag; support is tagged `2ND` (from `replay_scoring`'s per-frame role
  assignment; `--no-roles` to omit).
- **Skills** — a fading `★ <skill>` callout pops above the car that just
  performed a detected skill, and skills also stream in the ticker.
- **Possession** (current team, from the last touch), **per-player trails**, and
  blob **drop-shadows** for depth. Toggle trails / labels / boost / heatmap.
- **Heatmap** — a floor occupancy heatmap of the ball (or the followed player),
  binned in-viewer from the grid.
- **Momentum strip** — a P(blue scores the next goal) curve above the timeline
  (from `replay-value`; the model is basic, so read it as rough momentum, not a
  calibrated win probability). `--no-roles` / `--no-winprob` omit the overlays.

## Coordinates

The scene stays in Rocket League unreal units (Z-up: X side, Y length, Z height);
the web layer sets `DEFAULT_UP = (0,0,1)` so the camera and field read naturally.
Cars are oriented from `[pitch, yaw, roll]` (yaw about Z, then pitch, then roll),
with a white nose marking forward (+X) — exact orientation is a known area to
refine against footage.

## What's tested

The Rust scene projection, role attachment, downsample, and HTML/import-map
substitution are unit- and golden-tested (`viewer/tests/scene.rs`,
`golden.rs`). The JS/three.js rendering path is covered by a **headless-GL smoke
test** (`viewer/tests/gl_smoke.mjs`): it renders the `--offline` file under
swiftshader and asserts the scene drew (`window.__rendered`) with no page errors.
CI runs it (`.github/workflows/ci.yml`); locally, `npm i puppeteer` then
`node viewer/tests/gl_smoke.mjs <file.html>`.

## Follow-ups

Tracked in `backlog.md` (Viewer improvements). Remaining: validate car
orientation against footage; a win-probability / ΔV strip from `replay-value`;
and map-aware field geometry for non-standard arenas.
