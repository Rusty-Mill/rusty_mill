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
  earlier task. `routing`, when present, must be an object; a string,
  null, array, number, or boolean is rejected rather than silently
  falling back to the defaults. Refs reuse the adapter protocol's syntax through
  `orch_cli::parse_ref`, now public. Parsing lives in the binary with
  `rusty_json`; `orch-core` stays I/O-free.
- **Arguments are parsed by hand over `std::env::args`.** One subcommand
  and five flags do not justify a parser dependency. Every orch crate is
  registry-free, and `rush` sets the precedent.
- **Blocked questions.** Without `--interactive` the run stops with exit
  status 3 and the questions in the report. With it, each open question is
  offered on stderr and the answer read from stdin, appended as a Human
  `Answer`, and the dispatcher runs again. A blank line or end of input
  stops the round at once, whatever was answered before: earlier answers
  stay on the board, no further model call is made, exit status 3.
- **Channel discipline.** stdout is the report and nothing else, so
  `--json` is one parseable object even through a blocked-and-answered
  run. Questions, the answer prompt, and progress go to stderr. A progress
  line is built from an exhaustive match over the dispatcher's outcome:
  fixed categories plus task ids, agent names, roles, and counts. The
  text of an agent failure, which can carry model output or a child's
  stderr, never reaches it; the report holds it. Questions shown under
  `--interactive` are model text by nature. The entry point lives in the
  library over injectable streams so this is tested in-process.
- **Review context.** A review card's `refs` need not name the reviewed
  task's entries. The shared renderer (`orch-cli`) resolves the target's
  live entries and the answers to them at render time under
  `UNDER REVIEW`, after the refs and before the card's own history, with
  the same deduplication. Review verdicts are therefore always given on the
  actual output, including findings written after a question was answered.
- **Wall clock** is enforced in the binary, checked before each dispatcher
  run against `Budget::wall_clock`. The dispatcher itself still meters calls
  only; moving the deadline into it is a separate decision.
- **Output.** A text page or, with `--json`, one object with `goal`,
  `ended`, `calls`, `tasks`, and every board entry with a `superseded`
  flag. Progress lines go to stderr and carry only fixed categories, ids,
  and counts.
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
