---
category: Changed
changelog: CI scopes PRs that edit crate manifests and Cargo.lock together, or add workspace dependencies, instead of sweeping the whole workspace.
---
## 2026-10-25 - Scoped CI for manifest and lockfile changes

- **Changed:** a PR that edits member `Cargo.toml` files together with `Cargo.lock` is no longer a full-workspace sweep. `lockfile_diff.py --with-manifests` selects every workspace consumer of each lock entry that was added or changed; an entry that left the lock is explained by the edited manifests, whose crates are already selected by path. The lock records no features, so the planner also resolves the event base: if any package resolved in both has a different enabled-feature set (one member turning on a feature that unchanged members share), the full sweep stays. Any lock entry it cannot place in head metadata, or a base that fails to resolve, still falls back to the full sweep.
- **Changed:** `cargo_toml_diff.py` treats new `[workspace.dependencies]` keys as safe, like new members: an entry is inert until a member references it. Changing or removing an existing entry still forces the full sweep.
- **Why:** on 2026-10-25, #1018, #1024, #1026 and #1030 each ran the full 289-crate sweep (~60 min per queue entry) because of the old "Cargo.lock changed with a manifest" rule. Replayed against their real merges, the new planner selects 4, 98, 94 and 1 crates, with no feature drift on any of them.
- **Known limitations:** manifest edits *without* a lock change are still path-selected only, so a feature-only edit there can reach unchanged consumers unseen (unchanged by this PR). The daily full sweep covers it.
