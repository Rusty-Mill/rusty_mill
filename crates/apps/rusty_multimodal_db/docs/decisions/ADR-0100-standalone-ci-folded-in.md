# ADR-0100: The Standalone Workflow's Jobs Run in the Monorepo

- Status: **Proposed and implemented on one branch; the owner asked for
  it** (2026-09-22, "msrv"), the separate call `ADR-0098` named.
- Date: 2026-09-22
- Deciders: baileyrd
- Related: `ADR-0098` (the feature-set job this extends), `ADR-0043`
  (`ECO-FR-003` the client-only test run, `ECO-FR-008` the Python
  conformance vectors), `ADR-0092` (the `rust-version` 1.89 bump the
  MSRV job pins), the crate's `.github/workflows/ci.yml` (the
  standalone repository's workflow, which GitHub never reads inside
  the monorepo).
- Supersedes/Superseded by: none. Additive: steps on the monorepo's
  `multimodal-db-feature-sets` job and a new `multimodal-db-msrv` job;
  a note atop the standalone workflow; one manifest comment. No code
  change.

## Context

The crate arrived in the monorepo with its own workflow, and the
monorepo runs only root workflows. Four of its checks had no
counterpart: clippy per feature set (which is why `ADR-0092`'s and
`ADR-0094`'s breaks were invisible until `ADR-0097`), `cargo test
--features client`, the Python client's offline wire-vector
conformance test, and the MSRV check at the declared `rust-version`.
`ADR-0098` closed the first with `cargo check`; `--all-features` still
let dead code hide behind `research`-gated callers.

## Decision

Implement: the feature-set job runs `cargo clippy -- -D warnings`
under the default, `client`, `server`, and `research` feature sets
(a superset of `ADR-0098`'s check), then `cargo test --features
client`, the Python vectors, and `cargo doc --no-deps` (warnings
allowed, as the standalone workflow ran it); a `multimodal-db-msrv` job checks
`--all-features --all-targets` on `dtolnay/rust-toolchain@1.89.0`.
Both scoped by the affected-crate plan. The standalone workflow stays,
with a note that the root workflow is what runs here, for the mirror
repository where it is the whole CI. Every step verified locally
before the push: three clippy sets clean, client tests 263/263, the
six Python vector tests, `cargo doc` clean, `cargo +1.89.0 check -p rusty_multimodal_db --all-features --all-targets` clean.

## Consequences

- Positive: the crate's every standalone guarantee now gates the
  monorepo PR that would break it; the MSRV pin is checked where the
  crate actually lives.
- Negative / tradeoffs: one more cached job and four clippy passes
  per affected PR. The MSRV literal lives in three places (two
  workflows and `Cargo.toml`); the note and the comment say so.
- Named, not hidden: the standalone file is not deleted — a deletion
  is the owner's call, and the mirror repository still needs it.
  `cargo doc` is not run with `-D warnings`: the crate's rustdoc
  currently emits about 80 warnings (unresolved intra-doc links to
  items that moved between modules, mostly), so denying them would be
  red on day one — a "rustdoc clean" round is the follow-up this names.

## Acceptance and implementation

- 2026-09-22: implemented on `claude/pr-276-multimodal-db-growth-4lkjx8`. No code change; the suite unchanged.
  Builder: Claude; independent Codex inspection owed.
