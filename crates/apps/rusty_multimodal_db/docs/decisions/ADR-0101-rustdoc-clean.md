# ADR-0101: Rustdoc Clean — Warnings Denied

- Status: **Proposed and implemented on one branch; the owner asked for
  it** (2026-09-22, "rustdoc"), `ADR-0100`'s named follow-up.
- Date: 2026-09-22
- Deciders: baileyrd
- Related: `ADR-0100` (which found the 82 warnings and ran `cargo doc`
  with them allowed), `ADR-0043` (the doc build in the standalone
  workflow).
- Supersedes/Superseded by: none. Docs-only: intra-doc links in
  module and item docs; the monorepo's `cargo doc` step now sets
  `RUSTDOCFLAGS=-D warnings`. No code, wire, or behaviour change.

## Context

`cargo doc --all-features` emitted 82 warnings: 69 unresolved
intra-doc links, 12 public docs linking private items, one item it
could not document. Most had one cause: a module whose `pub mod` line
carries outer `///` docs has its *inner* `//!` links resolved by
rustdoc in the parent's scope, so `[`traits`]` written inside
`generic/mod.rs` was looked up at the crate root. The rest were
`[`Self::open`]` in module-level docs (no `Self` there), links to
private helpers from public docs, and a few paths that moved.

## Decision

Implement: qualify every inner-doc link in the affected modules from
the crate root (`crate::generic::traits`, `crate::server::serve`, …),
name the struct instead of `Self` in module docs, turn links to
private items into plain code spans, correct the moved paths, and
deny rustdoc warnings in the monorepo's doc step so the count stays
at zero. The rendered docs gain working links; the prose is
unchanged.

## Consequences

- Positive: every intra-doc link in the crate resolves; a future
  broken link fails the PR that breaks it.
- Negative / tradeoffs: inner-doc links are longer than they need to
  be in a module without outer docs; the qualified form is correct in
  either scope, so it is the safe convention going forward.
- Named, not hidden: the standalone workflow's own `cargo doc` step
  still allows warnings; it is the mirror repository's to change.

## Acceptance and implementation

- 2026-09-22: implemented on `claude/pr-276-multimodal-db-growth-4lkjx8`. `RUSTDOCFLAGS=-D warnings cargo doc -p
  rusty_multimodal_db --all-features --no-deps` clean (0 warnings, down
  from 82); `cargo fmt` check clean; `cargo clippy --all-features
  --all-targets -- -D warnings` clean. Builder: Claude; independent
  Codex inspection owed.
