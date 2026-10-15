# Super-board autonomy policy

**Status: ACCEPTED by the owner on 2026-10-15, as written** (every recommended default
below is the decision). It takes effect when the PR that carries this file merges. Changes
to it need a reviewed PR.

**Owner direction (2026-10-15):** automatic merging is the point. No category of change is
routed to a human by default, and the policy does not require an independent reviewer. The
safeguards below are mechanical (CI, target, size, kill switch), not human.

## Facts this draft rests on

- `main` is protected by a ruleset: PR required, **merge** method only, required check
  `required-gate`, merge queue, **0 required approvals, no bypass actors**. So a robot PR
  can reach `main` with no human, subject to a green `required-gate` on the queued entry.
- `staging` is **not** protected and has no required checks.
- The merge gate merges with `gh pr merge`. On a queue-protected branch that should enqueue
  the PR rather than merge it directly. **This is unverified** until a real card runs.
- The repo has **no database migrations**.
- CONTRIBUTING.md asks for independent review of security-sensitive, irreversible and
  ecosystem-breaking changes. This draft drops that for robot PRs, so CONTRIBUTING must be
  amended in the same PR, or the two documents disagree.
- GitHub gives 5,000 API points per hour; one card costs roughly 950 on a busy board.

## What `staging` would buy, and what it costs

| | If protected like `main` and the robot merges to it | If dropped (robot merges to `main`) |
|---|---|---|
| Benefit | A buffer: `main` only changes when staging is promoted, so a bad robot merge never reaches `main` on its own | One path, one set of checks, no divergence |
| Needs | A gate between `staging` and `main`: a person, a schedule plus extra tests, or both. **Without one, `staging` adds nothing**: it is `main` one step behind | Nothing extra |
| Cost | Second branch to keep in sync, double CI, a promotion PR to merge | A bad merge lands on `main` (the queue and `required-gate` still apply; revert is the recovery) |

Since no human gate is wanted, **recommended: drop `staging`** and have the robot merge to
`main` through the queue. Keep `staging` only if you want a promotion step.

## Decisions

| # | Question | Recommended default | Why | Decision |
|---|---|---|---|---|
| 1 | Who merges? | **The robot**, through the merge gate, after review passes | Owner direction | Accepted |
| 2 | Which branch do PRs target? | **`main`**, via the merge queue; delete `staging` | See the table above; `staging` is unprotected today | Accepted |
| 3 | Migrations | None allowed (`allowed_envs: []`); never a live database | No migrations exist | Accepted |
| 4 | Commands the agent may run without asking | The lanes' own commands, **narrow rules only**; no wildcard `git push` or `gh api` | A wildcard covers far more than a lane needs | Accepted |
| 5 | Pushes | Only to `claude/*` branches; never `main`, `master` or `staging` | The push guard denies protected branches | Accepted |
| 6 | Tickets the robot may take | Cards in **Ready** only, labelled `feature`, `bug` or `qa`, lint-clean | A card nobody moved to Ready never starts | Accepted |
| 7 | Merge conditions (all mechanical) | Green `required-gate` for the exact head; target is `main`; allowed merge method; PR under 400 changed lines; no failed truth-check | The gate enforces each one and fails closed | Accepted |
| 8 | Pace | `max_workers: 2`; `no_progress_cycles: 6`; `usage_pause_pct: 95` | Cost ceiling and a runaway stop | Accepted |
| 9 | Stopping | `/super-board stop` any time; the halt file stops everything on a GitHub read failure | A kill switch | Accepted |
| 10 | Recovery | A bad merge is reverted by PR; two reverts in a day pauses the robot | Makes the cost of being wrong small and visible | Accepted |

## Rollout

| Step | Robot may | Next step when |
|---|---|---|
| 1. Pilot | Run **one** card (#685) with the owner watching, merging through the queue | It merged cleanly and the owner agrees the PR was fine |
| 2. Steady | Take Ready cards automatically within the conditions above | Ongoing; any revert triggers decision 10 |

## What accepting changes in the repo (done in the PR that carries this file)

1. Done: `.claude/super-board/configs/rusty-mill.json` added (`base_branch: "main"`,
   `merge_policy.default: "auto"`, `method: "merge"`, `allowed_envs: []`, empty `always_human`,
   `ci_required_checks: ["required-gate"]`, `max_workers: 2`).
2. Not committed, by design: permission rules. Each person who runs the board adds the
   narrow set from [proposed-settings.md](proposed-settings.md) to their own
   `.claude/settings.local.json`. Without them the agent asks before each command.
3. Done: CONTRIBUTING.md and [AGENTS.md](../../AGENTS.md) amended (no required independent
   review for robot PRs; the tooling is active).
4. Owner action after merge: delete `staging` (`git push origin --delete staging`; the push
   guard blocks it, so the owner runs it).

## Risks the owner accepts

- A robot PR touching crypto, TLS or auth merges without a human reading it. Several of those
  crates are marked "review pending", and tests plus a green gate do not prove a crypto change correct.
- The gate has been tested against fakes, not a real card. Step 1 exists to find what the
  fakes missed.
- The robot's review is by the same model family that wrote the code, so it can share its blind spots.
