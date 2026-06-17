# 3D Replay Viewer — `replay-viewer`

> Play a reconstructed match back in 3D in the browser — a broadcast-grade render
> (shadows, painted pitch, goal nets) with analysis overlays (1st/2nd-man roles,
> skills, heatmap, momentum, per-touch impact), coaching tools (field overlays + a
> telestrator), and
> the usual scrubable timeline, camera presets, and event/skill ticker.

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
goal · `k`/`j` next/prev kickoff · speed (0.25–4×) · loop · `s` save PNG ·
`?` help · mouse drag orbits, scroll zooms, right-drag pans (touch works too).

**Camera:** `overview` / `goal` / `ball` / `tv` (broadcast) presets with eased
transitions, or click a player row to lock onto that car (`0` returns to free
overview). Goal/demo/kickoff markers on the timeline are clickable, as is each
ticker entry — both seek.

## Look

Filmic (ACES) tone mapping, real soft shadow maps, a painted pitch (lines, centre
circle, goal areas, mow stripes), goal nets, four-wheeled cars, a gradient sky,
a rolling ball, boost flames, and a demo burst — a broadcast-grade render.

## Overlays

- **Scoring roles** — the 1st man on each team wears a gold ground-ring and a
  `1ST` HUD tag; support is tagged `2ND` (from `replay_scoring`'s per-frame role
  assignment; `--no-roles` to omit).
- **Skills** — a fading `★ <skill>` callout pops above the car that just
  performed a detected skill (tinted by the touch's value swing when it is a ball
  contact), and skills also stream in the ticker.
- **Possession** (current team, from the last touch), **per-player trails**, a
  top-down **minimap**, and a roster with header **G/A/Sv** + live **speed**.
  Toggle trails / labels / boost / heatmap.
- **Boost readout** — each car carries a tinted **boost-amount number** floating
  above it (green/amber/red by level), alongside the existing boost gauge bar;
  both follow the `boost` toggle.
- **Live scoreboard** — the score **counts up with the playhead** (goals up to the
  current time), so scrubbing shows the score as it stood, not the final result.
- **Heatmap** — a floor occupancy heatmap of the ball (or the followed player),
  binned in-viewer from the grid.
- **Momentum strip** — a P(blue scores the next goal) curve above the timeline
  (from `replay-value`; the model is basic, so read it as rough momentum, not a
  calibrated win probability). `--no-roles` / `--no-winprob` omit the overlays.
- **Impact (ΔV)** — each ball-contact touch is credited the scoring-probability
  swing it caused (from `replay-value`'s per-touch ΔV): the roster carries a
  per-player **impact** chip (total ΔV, green helped / red hurt), touches in the
  ticker show their signed swing, and ball-contact skill callouts are tinted by it.
  Read it as a rough impact signal (same basic model). `--no-impact` omits it.

## Coaching tools

- **Field overlay** — break down spaces by projecting a layout on the floor:
  built-in presets (`thirds` defensive/mid/attacking · `lanes` left/center/right ·
  `grid`), or **your own image** (the `img…` button, or drag-and-drop an image onto
  the view) with an opacity slider. The six big boost pads are marked.
- **Telestrator** — a coach's pen over the view: toggle `draw` (key `d`), pick a
  colour/width and `pen` / `arrow` / `line`, and annotate. Strokes persist on
  screen until `clear`; `undo` (key `z`) removes the last. Drawing pauses playback
  and frees the mouse from the camera.

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
orientation against footage; and map-aware field geometry (Phase 2) for
non-standard arenas.
