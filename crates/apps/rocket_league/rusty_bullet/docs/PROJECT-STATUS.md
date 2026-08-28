# Project Status

- Last verified main commit: *(pending first push — see note below)*
- Verified at: 2026-08-28
- Current milestone: `PHASE-0-BOOTSTRAP` (repo lifecycle bootstrap)
- Health: green — bootstrap complete, workspace builds and tests pass

## Completed

- `PHASE-0-BOOTSTRAP` — charter, system architecture, spec tree (6 specs),
  spec registry, 3 ADRs, research backlog, roadmap, traceability, AGENTS.md/
  WORKFLOW.md, governance file set (README/CONTRIBUTING/CODE_OF_CONDUCT/
  SECURITY/CHANGELOG/RELEASE_NOTES/PR & issue templates), CI workflow, and a
  minimal buildable Cargo workspace (`rb_domain`, `rb_replay_ingest`,
  `rb_capture_ingest`, `rb_verify_cli`) with the divergence-scoring
  algorithm implemented and unit-tested. Evidence: this commit, on
  `claude/rocket-league-server-clone-u74q45` (not yet merged to `main`).

## In progress

- None — bootstrap work is complete pending review/merge of this branch.

## Blocked

- None.

## Next

1. `PHASE-0-REPLAY-INGEST` — add `boxcars` as a dependency of
   `rb_replay_ingest` and implement `RB-VERIFY-001` against a real replay
   file from the owner's own match history.

## Validation

- `cargo fmt --all -- --check`: pass
- `cargo clippy --workspace --all-targets -- -D warnings`: pass
- `cargo test --workspace`: pass (11 tests: 6 in `rb_domain`, 1 each in
  `rb_replay_ingest`/`rb_capture_ingest`, 0 in `rb_verify_cli` (binary-only,
  no unit tests yet), plus doc-tests)

## Risks and decisions needed

- `RB-RESEARCH-O001` (build vs. integrate physics) — decide once
  `PHASE-0-EXIT` produces real divergence data. Owner: baileyrd.
- `RB-RESEARCH-O002` (binary reverse engineering) — needs explicit owner
  sign-off after legal/practical review before any work starts. Owner:
  baileyrd.
- `RB-RESEARCH-O003` (capture tooling scope) — decide at
  `PHASE-0-CAPTURE-INGEST` start. Owner: baileyrd.
- This is a fresh repo with no `main` history yet; the note above about
  "last verified main commit" should be filled in once this bootstrap
  branch is merged.
