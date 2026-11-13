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
  **One exception:** the PR queue watcher also works the queue and queues and merges
  PRs (owner, 2026-11-06 "you merge too", confirmed 2026-11-10). It follows every
  merge rule here; no other agent queues or merges.
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

## New work: notes first, cards on commitment

Owner decisions of 2026-11-12, Millwright E1 and M4 (`entry-rule` and `backlog-commit`
in [docs/projects/millwright/DECISIONS.md](docs/projects/millwright/DECISIONS.md)):

- A new idea starts as a design note in remind-me. It does not start as Mill Board cards
  or a new repository.
- Create a repository or cards only after an overlap check against the existing
  repositories and board projects, and only for committed work.
- Work is committed when its project has a priority in
  [PRIORITIES.md](docs/projects/millwright/PRIORITIES.md). Agents pull from those
  priorities, never from card priorities they set themselves. A paused project takes
  no new cards.
- A deliberate experiment is allowed when it is labelled `experiment`. It ends with a
  harvest: a one-page conclusion in remind-me.

## Board identity: label your instance

Agents that share one Mill Board credential or connector (for example every cloud session as
`claude-cloud`) pass `actor` as `<credential>/<label>`, label = your thread or session id, on
every write: `claude-cloud/7x5uhx`. Claims, history and comments then tell instances apart.
The credential still fixes your role and projects; the label is attribution, not authority.
Use one label for the whole task, since a claim only matches the exact name that made it.

Tickets: [issue template](docs/agents/issue-tracker.md). The tooling lives in `.claude/`.
