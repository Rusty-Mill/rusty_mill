---
category: Fixed
changelog: CI runs the full sweep when a crate's manifest changes its dependency or feature declarations, since that can change how unrelated crates build.
---
## 2026-10-25 - Member dependency edits keep the full CI sweep

- **Fixed:** a PR that edits a member `Cargo.toml` without touching `Cargo.lock` was path-selected: only the edited crate and its reverse dependencies ran. Cargo unifies a shared package's features across the build, so a member that declares a dependency or feature differently (including per target, or as a build or dev dependency) can change how crates that never depend on it build. `cargo_toml_diff.py --members` now compares each changed member manifest's dependency tables, per-target dependency tables and `[features]` with the event base, and any difference, or a new member, keeps the full sweep. Edits elsewhere in a manifest (`[package]`, `[lib]`, `[lints]`, ...) stay narrowed.
- **Cost:** of the 104 merges to `main` from 2026-10-23 to 2026-10-25, five edited member manifests without a lock change, and three of those (#1027, #1022, #968) changed dependency declarations. Those would now run the full sweep.
