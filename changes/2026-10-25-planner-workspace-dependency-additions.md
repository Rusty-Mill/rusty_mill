---
category: Changed
changelog: CI scopes PRs that only add new workspace dependency entries to the root Cargo.toml instead of sweeping the whole workspace.
---
## 2026-10-25 - New `[workspace.dependencies]` keys no longer force a full sweep

- **Changed:** `cargo_toml_diff.py` treats new `[workspace.dependencies]` keys in the root `Cargo.toml` as safe, like new `[workspace.members]`: a key is inert until a member's manifest references it, and that manifest is selected by path. Changing or removing an existing key still forces the full sweep.
- **Unchanged on purpose:** a PR that edits member manifests together with `Cargo.lock` still runs the full sweep. The lock records no features, and the feature-unification effects of a manifest edit (including target- and dependency-kind-specific activation) cannot be shown equivalent cheaply, so narrowing those was dropped after review.
