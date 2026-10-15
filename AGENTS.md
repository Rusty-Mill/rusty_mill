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
- Never force push or rebase shared history. Never read `.env` or commit/log secrets.
- Validate the affected scope using repository checks; a local `cargo check` alone
  does not establish green CI or merge readiness.

## Super-board — present, inactive

The super-board skills, scripts, hooks, workflows and tests are in `.claude/`, but nothing
activates them. No permission rules and no auto-run hooks are committed; the guard hooks in
`.claude/settings.json` only deny. The board config is documentation at
[proposed-config.md](docs/super-board/proposed-config.md), outside the discovery path, so
`/super-board run` and the merge gate have no config to run from. Do not add one.

The proposed [permissions and session hook](docs/super-board/proposed-settings.md) are
proposals, not authority. Their writing standards, ticket format, block templates and
PR author notes ship with the tooling and are not required by the workflow above. The
standalone [issue template](docs/agents/issue-tracker.md) is documentation.

These docs do not authorize activation, automatic merges, migrations, permission changes
or credential operations, including through local copies of the config or settings.
Activation needs an independent review of the tooling and an explicit bounded owner policy
decision on autonomy, permissions and migrations. `/super-review` and the super-board merge
gate stay out of the workflow above until then.
