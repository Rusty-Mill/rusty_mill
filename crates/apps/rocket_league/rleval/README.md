# RLEvalSystem

A scoring-agnostic Rocket League replay analysis workspace: decode a `.replay`
into a neutral **canonical match model**, then derive higher-level views on top
of it. The canonical model is the contract; every other crate is a pure consumer
of it (no parsing, no I/O), so each is deterministic and golden-testable.

For day-to-day use the **`rleval` app** (the `app` crate) ties the whole
workspace into **one binary with a web UI** — drop a `.replay` and get the 3D
viewer, the scoring report, the skills table and the value-impact analysis side
by side, all computed in-process (no subprocess shelling). The individual crate
CLIs below remain for scripting and golden tests.

```sh
cargo run -p rleval-app -- serve          # then open http://127.0.0.1:8080
cargo run -p rleval-app -- analyze game.replay --out game.html   # one-shot bundle
```

## Workspace crates

- **`app`** (`rleval`) — the **unified application**: a single binary that serves
  a single-page web UI (a tiny dependency-free HTTP server) and runs every engine
  below in one in-process pipeline, returning the 3D viewer, scoring report,
  skills and value-impact for a replay in one page. `rleval serve` (web UI) or
  `rleval analyze <replay>` (a self-contained static HTML bundle).
- **`replay-analyzer`** (root) — decode (`boxcars` adapter behind a swappable
  `ReplayParser` port) → reconstruct world state → coalesce identity-stable
  player tracks → fixed-rate resample → derive events (touches, possessions,
  kickoffs, demos, goals) → the `CanonicalMatch`. Knows nothing about scoring.
- **`scoring`** — pure, config-driven **decision-discipline** scoring
  (positioning/rotation): roles → metrics → composite + sub-scores → licence
  band → leak→chapter, plus lobby report + SVG heatmaps + HTML
  (`replay-scoring`).
- **`skills`** — mechanical **skill detection & verification**: a 12-skill
  catalog detected from the canonical grid/events, and an API to verify whether
  a skill was performed (by player, within a time window). The complement to
  scoring — *mechanics, not positioning* (`replay-skills`). See
  [`docs/skill-detection.md`](docs/skill-detection.md).
- **`value`** — a calibrated value model used as an independent validator of the
  scoring rubric (two-track reconciliation).
- **`viewer`** — a self-contained **3D replay viewer**: animates the
  reconstructed match (ball + cars, boost, name labels) in the browser with a
  scrubable timeline and an event/skill ticker (`replay-viewer`). See
  [`docs/replay-viewer.md`](docs/replay-viewer.md).

## Quickstart

```sh
# Canonical model (JSON) from a replay
cargo run -p replay-analyzer -- assets/replays/42f2.replay --json out.json

# Decision-discipline report (per player / lobby HTML)
cargo run -p replay-scoring -- assets/replays/42f2.replay --html report.html

# Mechanical skills: catalog, per-player summary, and verification
cargo run -p replay-skills -- --list
cargo run -p replay-skills -- assets/replays/42f2.replay
cargo run -p replay-skills -- assets/replays/42f2.replay --verify aerial --player Schutzein

# 3D replay viewer (writes a self-contained HTML file; open in a browser)
cargo run -p replay-viewer -- assets/replays/42f2.replay --html 42f2.html
```

See [`docs/`](docs) for the design spec, backlog, and per-feature notes —
including [`docs/detection-catalog.md`](docs/detection-catalog.md), a full
reference of everything detectable in a replay across all crates.
Scores and skill detections are **heuristic** inferences from kinematics
(replays carry motion, not inputs); thresholds are versioned and want corpus
calibration before the absolute numbers are trusted.
