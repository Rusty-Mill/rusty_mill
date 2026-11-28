---
category: Added
changelog: Let Em Loose #1319 slice 1, the `lel_treegen` core - a frozen seeded PRNG, generation profiles, the pure tree planner and the canonical manifest with its sha256.
---
## 2026-11-28 - LEL #1319 lel_treegen core

- **Added:** `crates/tools/lel_treegen`, workspace dependencies only (`clap`, `serde`, `serde_json`, `sha2`). A hand-written SplitMix64/xoshiro256** PRNG pinned to reference vectors. `Profile` holds the built-in default (integer weights, so no platform float drift) or one derived from a `lel-drive-stats/2` report. `plan(seed, scale, profile)` is pure and lays out units keyed by index, so a smaller scale is an exact prefix of a larger one. Fixed edge cases come first in every slice: 50 links into `outside/`, 20 hardlink pairs, zero-byte, unreadable, Unicode, 255-byte, over-260-character and depth-40 entries. `manifest::render` writes canonical JSON Lines (header, entries, expected groups); a golden digest test pins the algorithm. `lel-treegen plan` writes `manifest.jsonl` and `manifest.sha256` into a new directory.
- **Known limitations:** No tree bytes are written yet; `write`, `delta` and `verify` and the PNG/JPEG writers come in slice 2. Duplicate rates from a stats file stay at the defaults, because the stats pass cannot measure them by kind.
