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
- **Agents never queue or merge PRs.** DottiusMaximus (`dottiusmaximus`) manages reviews
  and merges (owner decision, 2026-10-27). An agent opens the PR, drives CI green,
  requests review from `dottiusmaximus`, and stops there.
- Merges go through the merge queue using "Merge when ready", wait for `required-gate`
  on the queued entry, and use a **merge commit**, never squash or rebase merge. Do not
  bypass red or stuck checks.
- Merge the default branch into a PR branch only for a merge conflict, a required-check
  failure caused by `main`, or an explicit owner request; never just to update it. The
  merge queue tests against current `main`.
- Never force push or rebase shared history. Never read `.env` or commit/log secrets.
- Validate the affected scope using repository checks; a local `cargo check` alone
  does not establish green CI or merge readiness.

## Super-board — merging retired

The board robot follows [docs/super-board/autonomy-policy.md](docs/super-board/autonomy-policy.md),
except that its merge decision is superseded by the rule above (owner, 2026-10-27):

- It takes Ready cards, builds, QAs and reviews them, and opens PRs against `main`. It does
  **not** queue or merge them; they wait for DottiusMaximus like any other PR.
- It never pushes to `main`, `master` or `staging`, and never runs a database migration.
- Permission rules are not committed. Each person adds the narrow set from
  [proposed-settings.md](docs/super-board/proposed-settings.md) to `.claude/settings.local.json`.
- Stop it with `/super-board stop`. A bad merge is reverted by PR.

Tickets: [issue template](docs/agents/issue-tracker.md). The tooling lives in `.claude/`.
