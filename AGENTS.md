# Agent instructions

## Current contribution workflow

Follow [CONTRIBUTING.md](CONTRIBUTING.md), including its review and merge rules.
Keep changes focused, preserve unrelated work, and add or update the PR's
[change fragment](changes/README.md). Use the existing PR templates.

- Every change lands through a PR; never push directly to the default branch.
- Require green CI for the current PR head and review of that diff. Obtain an
  independent approval when reasonably available; otherwise follow CONTRIBUTING's
  documented self-review procedure after CI is green. Record self-review honestly.
  Security-sensitive, irreversible and ecosystem-breaking changes follow its
  independent-review guidance.
- Merge through the merge queue using "Merge when ready" and wait for
  `required-gate` on the queued entry. Use a **merge commit**, never squash or
  rebase merge. Do not bypass red or stuck checks.
- Merge the default branch into a PR branch only for a merge conflict, a required-check
  failure caused by `main`, or an explicit owner request; never just to update it. The
  merge queue tests against current `main`.
- Never force push or rebase shared history. Never read `.env` or commit/log secrets.
- Validate the affected scope using repository checks; a local `cargo check` alone
  does not establish green CI or merge readiness.

## Super-board — active under the accepted autonomy policy

The board robot is governed by [docs/super-board/autonomy-policy.md](docs/super-board/autonomy-policy.md)
(accepted by the owner, 2026-10-15). In short:

- It takes Ready cards, builds, QAs and reviews them, and **merges its own PRs** into `main`
  through the merge gate and the merge queue, after a green `required-gate` for the exact head.
- It never pushes to `main`, `master` or `staging`, never merges a PR that targets another
  branch, and never runs a database migration. A migration PR waits for a person.
- Its PRs are exempt from the independent-review expectation above. Human-authored PRs are not.
- Permission rules are not committed. Each person adds the narrow set from
  [proposed-settings.md](docs/super-board/proposed-settings.md) to `.claude/settings.local.json`.
- Stop it with `/super-board stop`. A bad merge is reverted by PR.

Tickets: [issue template](docs/agents/issue-tracker.md). The tooling lives in `.claude/`.
