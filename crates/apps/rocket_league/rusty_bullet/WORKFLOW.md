# Repository Development Workflow

## Authority

`main` is authoritative. Chat history, task descriptions, local branches,
and reports are proposals or working state until merged.

## Executor detection

Detected fresh each session from environment capabilities, never from a
repository-stored flag. See
`rust-repo-lifecycle/references/executor-modes.md` for the full detection
logic:

- **Claude mode** — the session has its own shell, repository access, and
  GitHub access, and can implement, commit, open a PR, and merge itself. Run
  the outer and inner loop in one continuous session.
- **ChatGPT+Codex mode** — the session is a planner/reviewer with no shell
  of its own, relaying paste-ready instructions to a human who runs Codex
  and reports back. Use the trigger-phrase protocol (`next`, `PR created`,
  `branch updated`, `Is it green yet?`) from `executor-modes.md`.

## Roles

- **Planner/reviewer** — repository-aware planner, instruction author, PR
  reviewer, correction author, merge gate.
- **Implementer** — bounded implementer and validator: either the same
  session (Claude mode) or a separate agent a human relays to (Codex mode).
- **Human** (Codex mode only) — coordinator who transfers prompts and
  opens/updates PRs.

## Source of truth

- Treat current `main` as authoritative.
- Read `AGENTS.md` and `docs/PROJECT-STATUS.md` plus their routed
  authorities (roadmap, spec registry, ADRs, traceability) before planning
  any unit of work.
- Inspect commits after the last recorded checkpoint in `PROJECT-STATUS.md`.
- Report conflicts between chat/task descriptions and repository state; do
  not rely on conversation memory over repository evidence.

## Outer loop

1. `next` — planner inspects current state (status, roadmap, registry, open
   PRs) and produces one complete implementation packet
   (`docs/PROJECT-STATUS.md` "Next" section, or a standalone packet using
   the schema in `rust-repo-lifecycle/references/artifact-contracts.md`).
2. (Codex mode: user relays it to Codex.) Implementer implements, validates,
   commits, and reports.
3. PR opened — `PR created`.

## Inner loop

1. Reviewer inspects the actual exact head, diff, scope, authorities,
   tests, docs, threads, and CI.
2. Pass → merge the exact reviewed head (merge commit — see
   [CONTRIBUTING.md](./CONTRIBUTING.md); never squash or rebase).
3. Otherwise → one correction packet; implementer updates the same branch;
   `branch updated`; re-review the new exact head.

## Safeguards

- Never merge failing, pending, missing, stale, or older-head CI.
- Restart review if the head changes.
- Don't begin a competing increment while a PR is active.
- Distinguish code failures from infrastructure/account failures.
- Don't silently expand scope or resolve authority conflicts — surface them.

## ADRs

Write one per delivery cycle during active major development (the regime
this project is in until the Phase 0-3 baseline is fully implemented);
taper to decisions-that-matter once the baseline is stable. See
`rust-repo-lifecycle/references/adr-cadence.md`. Template:
[docs/adr/TEMPLATE.md](./docs/adr/TEMPLATE.md).

## `next`

Verify merge, refresh `main`, reconcile `docs/PROJECT-STATUS.md`, select the
next dependency-ready roadmap unit from
[docs/roadmap/ROADMAP.md](./docs/roadmap/ROADMAP.md).
