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
- Explicit over implicit; type hints/annotations always.
- Flat control flow — guard clauses, early returns, avoid >3 levels of nesting.
- Short, single-purpose functions.
- Minimal dependencies — justify any new third-party one in the PR description.
- Never commit or log secrets/credentials. Validate external input at the boundary.
- Never silently swallow exceptions — handle, propagate with context, or log.

## Review & merge
- Every change lands through a PR — no direct pushes to the default branch.
- CI must be green before merge.
- Review: one approval from someone other than the author when an independent
  reviewer is reasonably available. When none is — this repo has a single
  maintainer most of the time — the author self-reviews instead:
  - re-read the whole diff against the reviewer checklist below, after CI is
    green rather than before, so the review covers what will actually merge;
  - say so in the PR description: that no independent reviewer was available
    and that the change was self-reviewed. Self-review is recorded as
    self-review, never represented as independent review.
  - security-sensitive, irreversible, or ecosystem-breaking changes (secret or
    credential handling, deletions, public-API breaks that cross crates,
    CI-control changes) should still wait for an independent reviewer when one
    can be found within a reasonable time.
- Reviewers, independent or self: check for scope creep, missing tests, and
  unexplained non-obvious decisions.
- Merge with a **merge commit** ("Create a merge commit" — merge and sync). Do **not**
  squash-merge or rebase-merge: full commit history is preserved deliberately.
