---
category: Removed
changelog: Remove rusty-remind-me from the monorepo; it is developed and released in Rusty-Mill/rusty-remind-me again (ADR-0013).
---
## 2026-10-28 - rusty-remind-me moves to its own repository

- **Removed:** `crates/apps/rusty_remind_me` (six crates) and everything that only served it: its workspace members and path dependencies, the `remind-me*` CI jobs and their `required-gate` entries, the planner's `remind_me` and `remind_me_legacy_import` flags, `REMIND_ME_PACKAGES`, `remind-me-release.yml`, and `.claude/tests/test_remind_me_install.py` (the installer it tested moved with the product). The product now lives in [`Rusty-Mill/rusty-remind-me`](https://github.com/Rusty-Mill/rusty-remind-me) with its history, released `v0.4.1` there, and builds against this repository's crates pinned to `libs-2026.10.0` (Mill Board #61).
- **Changed:** `.claude-plugin/marketplace.json` now sources the `rusty-remind-me` plugin from that repository, so installs through this marketplace keep updating. ADR-0004 is marked superseded. The README history and crate table, the workspace map and the memory sync contract's links follow.
- **Kept:** `.claude/hooks/remind-me-capture.py`, its settings registration and its tests, as this repository's cloud session capture hook.
- **Known limitations:** the vendored `aws-sigv4` patch (RUSTSEC-2026-0387) has no consumer here any more and Cargo lists it as unused. It is left for a separate reviewed change, as `vendor/README.md` requires for removing a patch. rusty-remind-me carries its own copy.
