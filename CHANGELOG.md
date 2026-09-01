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

### Changed
### Fixed
- `rusty_croc`'s four deprecated `GenericArray::from_slice` nonce
  constructions rewritten to the equivalent `From<&[T]>` conversion — the
  workspace resolves `generic-array` 0.14.9, where they fail this repo's
  `-D warnings` clippy gate

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
