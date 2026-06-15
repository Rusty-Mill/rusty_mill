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
  blob **drop-shadows** for depth. Toggle trails / labels / boost in the HUD.

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

Tracked in `backlog.md` (Viewer improvements). Remaining: vendor three.js for a
fully-offline file (+ a headless-GL CI smoke test); validate car orientation
against footage; heatmap floor projection; a win-probability / ΔV strip from
`replay-value`; and map-aware field geometry for non-standard arenas.
