# ADR-0002: An optional stage runner (`mill-runner`)

- **Status:** Accepted (built)
- **Date:** 2026-10-13

## Context
The board holds work and enforces who may move it, but nothing does the work. The BBP conductor
(ADR-0001) is paused while BBP is revised. A runner that needs no BBP is wanted now, and it must stay
optional: the board must not depend on it, and a board with no runner keeps working.

## Decision
A separate crate and binary, `rusty_mill_runner` / `mill-runner`, in the board's family (so it shares
the card types, and the layer check allows it). It is a client of the board's HTTP API.

- **Stage table.** A JSON file lists stages: source columns, an optional column to enter, a board
  role, a command (argv, no shell), and the column after success and after failure. The default shape
  is build (Ready/Building -> Qa) then test (Qa -> Review, or back to Building). Any command works: an
  agent CLI, `make test`, a script. No vendor is built in.
- **Humans keep the gates.** The runner is given only the capabilities its stages need (checked at
  startup against the board's roles), so it never triages or merges. Work enters at Ready by a person
  and leaves Review by a person.
- **One card, one worktree.** Each card gets a git worktree on its own branch `mill/<id8>`. The
  command runs under `rusty_sandbox` (Landlock, rlimits, wall-clock group kill, clean environment):
  it can write only that worktree and a scratch directory, and read a short list of roots. Internet
  sockets are off unless the stage sets `network` (an agent CLI reaching its model). The runner
  commits afterwards, outside the sandbox, with an explicit `--git-dir`, `--no-verify` and no hooks, so
  the command never has `.git` writable and cannot redirect git.
- **Stateless loop.** Each tick lists the board and fixes what it finds, as the GitHub sync does. A
  card the runner itself holds claimed (it crashed mid-run) is resumed. Later stages and later columns are
  served first, so rework and work in progress finish before new cards start.
- **Feedback.** Every run is a comment (`[stage] attempt N: exit ..., output tail`). The next prompt
  includes the latest notes, so a retry sees why the last run failed. After `max_attempts` runs of a
  stage the card goes to Blocked with a comment. An infrastructure fault (git, sandbox) is an attempt,
  not a failure of the card: the claim is released and the card stays put.
- **Human override.** A holder of `Triage` may now move or release a card someone else claimed (board
  change). If a person moves a card while a command runs, the runner notices when it ends and changes nothing.

## Consequences
- The board gains `required_capability` (public), a Triage override on claims, and a shared blocking
  HTTP helper that GitHub sync now uses too.
- The runner is only as safe as its sandbox and its prompt source: a card body becomes a prompt, and
  cards can be imported from GitHub issues. Triage into Ready is the human check; the sandbox bounds the
  damage if it is wrong. Secrets passed with `--env` are visible to the command.
- Not done: killing a command as soon as a person moves its card (only noticed at the end, bounded by
  `wall_secs`); worktree cleanup after merge; per-role credentials, so the runner's role is still a
  claimed name; a pull-request step.

## Alternatives rejected
- **Run commands inside the board process.** Makes the board depend on process management and a
  sandbox for a feature most boards will not use.
- **A shell string per stage.** A card body flows into the command line; argv with placeholders keeps
  card text out of any shell.
- **Letting the command write `.git` and commit itself.** The agent could rewrite history on every
  branch, not just its own.
