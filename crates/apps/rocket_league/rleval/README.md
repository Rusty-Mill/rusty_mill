# RLEvalSystem

A scoring-agnostic Rocket League replay analysis workspace: decode a `.replay`
into a neutral **canonical match model**, then derive higher-level views on top
of it. The canonical model is the contract; every other crate is a pure consumer
of it (no parsing, no I/O), so each is deterministic and golden-testable.

## Workspace crates

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
```

See [`docs/`](docs) for the design spec, backlog, and per-feature notes.
Scores and skill detections are **heuristic** inferences from kinematics
(replays carry motion, not inputs); thresholds are versioned and want corpus
calibration before the absolute numbers are trusted.
