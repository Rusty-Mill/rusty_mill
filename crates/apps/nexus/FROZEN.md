# Nexus is frozen

Frozen 2026-10-07. Nexus is not in use. It is kept as a source of code to
extract or retire, not a target for new work: no new integrations or
adapters (`nexus-agui` was removed in PR #531).

## What "frozen" means

- Its 41 crates are in the root `Cargo.toml` `exclude` list, not
  `[workspace] members`. `cargo build/test/clippy --workspace`, CI, the
  workspace map and `Cargo.lock` no longer include them.
- Nothing outside `crates/apps/nexus` depends on a Nexus crate.
- Nothing is deleted. Source, docs, the pnpm tree and `shell/` are untouched.

## Unfreeze

Revert the freeze commit (it only moves the member lines, prunes
`Cargo.lock` and regenerates `docs/WORKSPACE-MAP.md`), or move the
`crates/apps/nexus/crates/*` lines back into `members` and regenerate both
(`cargo metadata --format-version=1 --all-features --locked`, then
`.github/scripts/generate_workspace_map.py`).

## Retire

Deleting the tree is a separate, explicitly approved change. See
`docs/research/CONSOLIDATION-AUDIT.md` §3 for what is generic enough to
extract first (nothing, unless a second consumer appears).
