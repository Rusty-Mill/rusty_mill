---
category: Added
changelog: Scope for a first-party Rocket League replay parser replacing boxcars and subtr-actor (about 4,350 lines in six PRs).
---
## 2026-10-24 - Rocket League: own replay parser, scope

- **Added:** `rleval/docs/replay-parser-scope.md`, sizing the first-party `.replay` parser the owner chose over `boxcars` (board w7): a Tier S `rl_replay` crate behind the existing `ReplayParser` port, about 3,350 lines for the parser and 1,000 more to move `replay-analyzer` and `rb_replay_ingest` onto it. `boxcars` and `subtr-actor` stay as dev-only differential oracles. Six PRs, the fifth adding the replicated input hints input recovery (r6) wants.
- **Clarified:** proposed oracle use moves into tests/examples, including both `recon-check` runners and `corpus_check`; future acceptance checks dependency kinds and normal/build paths, not `Cargo.lock`. `subtr-actor` 1.2.0 accepts `boxcars >=0.11.3, <1.0`; the workspace lock unifies 0.11.5.
- **Known limitations:** plan only, no code. Net version 10 and later only; after the switch, new Rocket League attributes are ours to add.
