---
category: Added
changelog: rl_replay, a first-party Rocket League replay parser with no external dependencies; its first slice parses the container, both CRCs and the header, equal to boxcars on every committed sample.
---
## 2026-10-24 - Rocket League: first-party replay parser, slice P1

- **Added:** `crates/apps/rocket_league/crates/rl_replay` (ADR-0002 Tier S, no external normal or build dependencies). `rl_replay::parse` checks both section CRCs, decodes the header (version triple, replay class, the full property tree) and returns the content section's bytes for the network decoder that later slices add. Errors fail closed and name what was being read and the byte offset.
- **Added:** a differential test against `boxcars` as a dev-dependency oracle: the container fields and the whole header of `42f2`, `419a` and the `rb_replay_ingest` sample equal boxcars' decode.
- **Changed:** `check_workspace_deps.py` pins `rl_replay` to no external dependencies.
- **Changed:** `rleval/docs/replay-parser-scope.md`: the version floor is 868.23.8, not net 10, because `419a.replay` is 868.23.8 and the old floor would have refused it. The bit reader moves to P2, where the network stream first reads bits.
