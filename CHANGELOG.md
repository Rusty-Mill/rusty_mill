# Changelog

All notable changes to **this monorepo itself** are documented here —
crate merges, workspace-wide CI, cross-crate changes. Each crate's own
internal changes are logged in that crate's own
`crates/<name>/CHANGELOG.md` where one exists (see ADR-0001 for why root
and per-crate logs are separate). Format: Added / Changed / Deprecated /
Removed / Fixed / Security, newest first.

## [Unreleased]
### Added
- `rusty_croc` merged into `crates/rusty_croc` via `git subtree` (fourth
  wave), full history preserved
- `rusty_test` merged into `crates/rusty_test` via `git subtree` (fourth
  wave) — six crates (`contract`, `compat`, `conformance`, `stat-tool`,
  `proc-runner`, `pty-shell`) behind one nested workspace
- `rusty_inventrory` merged into `crates/rusty_inventrory` via `git subtree`
  (fourth wave) — three crates (`inventory-core`, `inventory-cli`,
  `inventory-tauri`) behind one nested workspace
- `rusty_skillopt` merged into `crates/rusty_skillopt` via `git subtree`
  (fourth wave) — four crates (`skillopt-core`, `skillopt-model`,
  `skillopt-envs`, `skillopt-cli`) behind one nested workspace
- `rusty_key` merged into `crates/rusty_key` via `git subtree` (fourth
  wave) — eight crates (`rk-config`, `rk-observe`, `rk-constrain`,
  `rk-feed`, `rk-kernel`, `rk-mcp`, `rk-compose`, `rk-app`) behind one
  nested workspace; its Tauri desktop shell stays excluded, as upstream
  had it
- CI's Linux leg now installs `libwebkit2gtk-4.1-dev`, `libgtk-3-dev`,
  `libayatana-appindicator3-dev`, `librsvg2-dev`, and `libdbus-1-dev` for
  `inventory-tauri` and `inventory-core`'s Secret Service keyring backend

### Changed
- Root `[workspace.dependencies]`: `tokio`'s feature list widened to the
  union `rusty_search`/`rusty_db`/`rusty_skillopt` need (`fs`, `process`,
  `io-util`); `chrono` gained `serde` for `skillopt-core`

### Fixed
- `rusty_croc`'s four deprecated `GenericArray::from_slice` nonce
  constructions rewritten to the equivalent `From<&[T]>` conversion — the
  workspace resolves `generic-array` 0.14.9, where they fail this repo's
  `-D warnings` clippy gate
- `conformance`'s `layering.rs` layer-boundary check repointed at the
  monorepo root and scoped to `crates/rusty_test/` — it had located the
  workspace manifest two directories up and demanded a layer assignment for
  every member it found
- `inventory-core` pinned to `rusqlite = "0.32.1"` (from `"0.37"`): its
  `libsqlite3-sys ^0.35` requirement conflicts with `sqlx-sqlite`'s
  `^0.30.1` on the `sqlite3` `links` key, the same constraint `rusty_sqlite`
  hit; 79 tests pass unmodified against the pin
- `inventory-core`'s three deprecated `GenericArray::from_slice` calls in
  `db.rs` rewritten, same `generic-array` 0.14.9 cause as `rusty_croc`'s
- `rusty_key`'s eight crates keep a literal `[lints.rust] unsafe_code =
  "forbid"` instead of inheriting this root's `[workspace.lints]`, which is
  `rustils`' weaker `"warn"` — inheriting would have silently downgraded
  the policy
- `crates/rusty_key` reformatted with `cargo fmt --all` (it was not
  fmt-clean under this workspace's settings)

## [workspace] - 2026-09-01
### Fixed
- `rusty_rdp`'s hand-rolled byte cursor deduplicated against `rusty_wire`
  ([#65](https://github.com/Rusty-Mill/rusty_mill/pull/65))
- Six workspace-wide duplication findings resolved (glob matching, SHA-1,
  `to_wide()`, `read_lines()`, Windows raw-mode flags, IFS splitting)
  ([#10](https://github.com/Rusty-Mill/rusty_mill/pull/10))

### Changed
- `rusty_ansder` split into itself (DER codec only) and a new `rusty_rag`
  crate (RAG/Q&A engine) ([#65](https://github.com/Rusty-Mill/rusty_mill/pull/65))

<!-- No version tags on this repo as such — "[workspace] - DATE" entries
     group changes to the monorepo's own build/governance surface, distinct
     from any per-crate version a crate under crates/<name> might carry on
     its own. Earlier crate-import history isn't backfilled here entry-by-
     entry; see RELEASE_NOTES.md's note on that and `git log --oneline
     --merges` for the full list. -->
