# ADR-0106: The Crate's Standalone Workflow Is Retired

- Status: **Proposed and implemented on one branch; the owner chose
  it** (2026-09-23, "2" — the deletion `ADR-0100` left as the
  owner's call). No code change.
- Date: 2026-09-23
- Deciders: baileyrd
- Related: `ADR-0098` (the feature-set job in the root workflow),
  `ADR-0100` (every standalone step folded into the root workflow;
  the file kept "for the mirror repository where it is the whole
  CI"), `ADR-0101` (rustdoc `-D warnings` in the root workflow).
- Supersedes/Superseded by: closes `ADR-0100`'s named follow-up.
  Deletes `.github/workflows/ci.yml` under this crate; nothing else.

## Context

GitHub reads workflows only under the repository root, so inside the
monorepo this file never ran; every job it described has run from the
root `.github/workflows/ci.yml` since `ADR-0098`/`ADR-0100`
(`multimodal-db-feature-sets`, `multimodal-db-msrv`, the shared fmt,
clippy, and test jobs, rustdoc with warnings denied since
`ADR-0101`). `ADR-0100` kept it for the crate's standalone mirror
repository, `baileyrd/rusty_multimodal_db`. That repository is
archived — last pushed 2026-09-10, before the monorepo line began —
and an archived repository runs no workflows. The file gated nothing
anywhere, and its MSRV pin was a third copy of a number that must
match `Cargo.toml`.

## Decision

Delete the file. The root workflow is the crate's CI; the MSRV
literal now lives in two places (`Cargo.toml` and the root
workflow's `multimodal-db-msrv` job) instead of three. If the mirror
is ever unarchived and needs CI of its own again, the last version of
the file is in this repository's history at the commit before this
one, and the root workflow's two crate-specific jobs are the current
statement of what to run.

## Consequences

- Positive: one fewer place for the MSRV pin to drift; no workflow
  that claims to be CI and is not.
- Negative / tradeoffs: an unarchived mirror would start with no CI
  until the file is restored from history — named, not hidden.
- Named, not hidden: the docs that cited the standalone file for
  "the pin" now cite the root workflow.

## Acceptance and implementation

- 2026-09-23: implemented on `claude/pr-276-multimodal-db-growth-4lkjx8`. No Rust source touched; `cargo fmt -p rusty_multimodal_db -- --check` clean. The root workflow's own jobs are the verification, on the PR.
