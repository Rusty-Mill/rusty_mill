# ADR-0009: The command line

- **Status:** Accepted
- **Date:** 2026-10-03

## Context

Five library crates and two examples existed, but nothing a person could run
end to end without editing Rust. The examples hand-author their goal and
tasks in code. ARCHITECTURE listed goal intake as planned.

## Decision

- One binary, package and command both `rusty_orch`, at
  `crates/apps/rusty_orch/crates/rusty_orch`, with a library target so the
  loop and the parsers are testable without spawning the process.
- **Input is one JSON file.** Its top level carries `GoalDraft`'s fields by
  name, a `tasks` array of `TaskSpec` shapes, and an optional `routing`
  object. The task list is human-authored; the orchestrator does not ask a
  model to plan. `depends_on` and a review's `target` are indices into
  `tasks`, resolved to ids when the plan is built, and must point at an
  earlier task. Refs reuse the adapter protocol's syntax through
  `orch_cli::parse_ref`, now public. Parsing lives in the binary with
  `rusty_json`; `orch-core` stays I/O-free.
- **Arguments are parsed by hand over `std::env::args`.** One subcommand
  and five flags do not justify a parser dependency. Every orch crate is
  registry-free, and `rush` sets the precedent.
- **Blocked questions.** Without `--interactive` the run stops with exit
  status 3 and the questions in the report. With it, each open question is
  offered on stdout and the answer read from stdin, appended as a Human
  `Answer`, and the dispatcher runs again. A blank line stops.
- **Wall clock** is enforced in the binary, checked before each dispatcher
  run against `Budget::wall_clock`. The dispatcher itself still meters calls
  only; moving the deadline into it is a separate decision.
- **Output.** A text page or, with `--json`, one object with `goal`,
  `ended`, `calls`, `tasks`, and every board entry with a `superseded`
  flag. Progress lines go to stderr and never carry prompt or model text.
- **Adapters.** A composite over `CodexAgent` and `OllamaAgent` that
  forwards `run_classified` (ADR-0008). `Agent::Claude` and `Agent::Gemini`
  have no adapter and are refused permanently.

## Consequences

- The first real end-to-end artifact: a goal file produces a board.
- Persistence is still absent, so a run that stops blocked cannot resume in
  a later process. That is the next item.
- `StopRule::BestEffort` is parsed but behaves as `Checkpoint`; the
  dispatcher has no best-effort path yet.
- `orch-codex` gains a `[workspace.dependencies]` entry so the binary can
  depend on it the same way as its siblings.
