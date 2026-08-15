# Contributing

## Before you start
- Match surrounding conventions when editing existing code.
- Keep diffs focused — one logical change per PR.
- For large or hard-to-reverse changes (schema/data migrations, public API changes,
  deletions, dependency/toolchain bumps), open an issue or draft PR to discuss first.

## Workflow
1. Branch off the default branch.
2. Make your change. State the *why* in commit messages or PR description for any
   non-obvious decision.
3. Add tests for non-trivial logic — happy path and at least one failure/boundary case.
   Spikes/prototypes are exempt but should say so in the PR.
4. Add or update docstrings on any public surface you touched.
5. Open a PR — pick the template that matches (feature / bug fix / docs / chore).

## Code style
- `cargo fmt` and `cargo clippy --workspace --all-targets -- -D warnings` both
  clean before opening a PR — CI gates on each.
- Explicit over implicit. Prefer `Result` + `?` over panics; no `unwrap()`/
  `expect()` outside tests.
- Flat control flow — guard clauses, early returns, avoid >3 levels of nesting.
- Short, single-purpose functions.
- Minimal dependencies — justify any new third-party one in the PR description.
  See [`dependency-audit.md`](./dependency-audit.md) for how the existing set is
  classified; `libc` and `windows-sys` are a deliberate floor, and every other
  external crate here is either an interop contract or a documented exception.
- `unsafe` needs a `// SAFETY:` comment stating the invariant being upheld.
- Never commit or log secrets/credentials. Validate external input at the boundary.
- Never silently swallow errors — handle, propagate with context, or log.

## Review & merge
- Every change lands through a PR — no direct pushes to the default branch.
- CI must be green before merge.
- At least one approval required (see CODEOWNERS if present).
- Reviewers: check for scope creep, missing tests, and unexplained non-obvious decisions.
- Merge with a **merge commit** ("Create a merge commit" — merge and sync). Do **not**
  squash-merge or rebase-merge: full commit history is preserved deliberately.
