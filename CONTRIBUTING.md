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

## This repo's specifics

These override the generic guidance below where they differ. `CLAUDE.md` has the full version.

- **Merge with a merge commit, never squash or rebase.** Full history is preserved deliberately.
- **Merge only when CI is green** — all seventeen checks, not just the fast ones.
- **Run the full sweep before pushing, and check exit status.** Do not grep output for the word
  "error": a docs failure reached CI on #9 exactly that way — the command failed, its output did
  not contain what was being grepped for, and it read as a pass.
- **Do not write tests that race.** A test that merely *usually* observes the right ordering
  passes by luck on a fast machine and fails on a loaded runner. Make the window wide and fixed
  instead — `tests/ordering.rs` wraps the store in a decorator whose appends take 300ms.
- **When fixing a race, verify the new test fails without the fix.** Revert the source, run it,
  put it back. A concurrency test never seen to fail proves nothing.
- **Run the Redis and Postgres halves.** Several bugs have only ever appeared there, because the
  round-trips are slow enough to lose races the in-memory store wins by microseconds.
- **Comments and docs explain *why*, not what.** Where a choice had a real alternative, say what
  was rejected and what it would have cost.

MSRV is **1.86**. `redis-store` and `postgres-store` need 1.88 because their dependencies do; an
optional dependency does not raise the floor for everyone else.

## Code style
- Explicit over implicit; type hints/annotations always.
- Flat control flow — guard clauses, early returns, avoid >3 levels of nesting.
- Short, single-purpose functions.
- Minimal dependencies — justify any new third-party one in the PR description.
- Never commit or log secrets/credentials. Validate external input at the boundary.
- Never silently swallow exceptions — handle, propagate with context, or log.

## Review & merge
- Every change lands through a PR — no direct pushes to the default branch.
- CI must be green before merge.
- At least one approval required (see CODEOWNERS if present).
- Reviewers: check for scope creep, missing tests, and unexplained non-obvious decisions.
- Merge with a **merge commit** ("Create a merge commit" — merge and sync). Do **not**
  squash-merge or rebase-merge: full commit history is preserved deliberately.
