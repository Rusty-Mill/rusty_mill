# ADR-0098: CI Builds the Shipped Feature Sets

- Status: **Proposed and implemented on one branch; the owner asked for
  it** (2026-09-21, "ci gate"), after `ADR-0097` found two feature-set builds
  broken across several merged PRs.
- Date: 2026-09-21
- Deciders: baileyrd
- Related: `ADR-0097` (the finding), `ADR-0092`/`ADR-0094` (the two
  rounds that broke `--features server` and `--features client`
  respectively), `ADR-0072` and `ADR-0067` (the rounds whose test
  targets were never registered), the crate's own
  `.github/workflows/ci.yml` (the standalone repo's workflow, which
  the monorepo does not run — its `--features client` job would have
  caught the second break).
- Supersedes/Superseded by: none. Additive: one monorepo CI job, five
  `[[test]]` entries. No code change.

## Context

The monorepo builds this crate with `--all-features` only. A
deployment builds `--features server`; a client `--features client`;
`cargo build` alone neither. `ADR-0092` used a `research`-gated import
in shared code and `ADR-0094` left a `server`-only module ungated;
both merged green and both broke real builds until a local `cargo
check --features server` in `ADR-0097`. Separately, five integration
test targets that import `server::` were never given
`required-features = ["server"]`, so `--all-targets` under any lesser
feature set failed on them since `ADR-0067`/`ADR-0072`.

## Decision

Implement: a `multimodal-db-feature-sets` job in the monorepo
workflow — `cargo check -p rusty_multimodal_db --all-targets` under
the default, `client`, `server`, and `research` feature sets, scoped
by the same affected-crate plan every other job uses; and the five
`[[test]]` entries. Verified locally: all five feature sets check
clean, clippy `--all-features` clean, the suite unchanged.

## Consequences

- Positive: a feature-gating mistake fails the PR that makes it.
- Negative / tradeoffs: four more `cargo check`s per affected PR, a
  cached build each. The crate's standalone workflow remains
  unexecuted in the monorepo; folding its `msrv` and Python-vector
  jobs into the monorepo is a separate call.
- Named, not hidden: `check`, not `test` — the feature-set job proves
  the targets build, and `--all-features` still runs them.

## Acceptance and implementation

- 2026-09-21: implemented on `claude/pr-276-multimodal-db-growth-4lkjx8`. `cargo check -p rusty_multimodal_db
  --all-targets` clean under default, `client`, `server`, `research`,
  and `server,research`; `cargo clippy --all-features --all-targets
  -- -D warnings` clean; `cargo test -p rusty_multimodal_db --features server,research` — 962 tests across 43 targets, 0 failed, unchanged. Builder: Claude; independent Codex
  inspection owed.
