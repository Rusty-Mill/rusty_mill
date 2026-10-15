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

## Super-board — deferred and unavailable

The super-board skills, scripts, hooks, workflows and settings are not shipped
in this checkout. `/super-review` and the super-board merge gate are unavailable;
mandatory use of them is deferred, not a prerequisite for the workflow above.
The same applies to `/super-board`, `/super-build`, `/super-qa`, `/super-collect`,
`/git-sync`, `/ui-refine-loop` and `/visual` from that removed bundle.

Removed writing standards, ticket-format and block templates, PR author notes,
`super-board-pr-body.sh` and `super-board-env-check.sh` are deferred references,
not installed helpers or required commands. The standalone
[issue template](docs/agents/issue-tracker.md) remains available as documentation.

The [proposed configuration](docs/super-board/proposed-config.md) is inactive
reference material outside the operational discovery path. Its staging target,
automatic merge and test/staging migration settings are proposals, not established
authority. These docs do not authorize activation, automatic merges, migrations,
permission changes or credential operations, including through local tooling copies.
A separately reviewed implementation and an explicit bounded owner policy decision
are required before any activation. Do not restore tooling to satisfy these docs.
