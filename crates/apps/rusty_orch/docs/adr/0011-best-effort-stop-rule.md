# ADR-0011: Best effort means no questions, assumptions instead

- **Status:** Accepted
- **Date:** 2026-10-04

## Context

`StopRule` has two variants since the goal contract was written:
`Checkpoint`, "halt and report back for a decision", and `BestEffort`,
"continue, recording assumptions on the blackboard". The rule is a required
field of every goal file, yet nothing read it: ADR-0009 recorded that
BestEffort behaved as Checkpoint. `EntryKind::Assumption` existed for
exactly this case ("taken as true without verification, best-effort
runs") and Research, Triage, and Design cards could already write it, but
nothing told an agent when to assume rather than ask, and any `question`
in a reply blocked the card whatever the rule said.

The stop rule is a goal-level fact. `AgentRunner::run` receives the card
and the board, not the goal, and the shared renderer and parser in
`orch-cli` receive the role. The binary knows the goal when it builds the
adapters.

## Decision

- **The rule is enforced by the adapters, not the dispatcher.** Under
  `BestEffort`, `orch_cli::allowed_kinds` withdraws `question` from
  Research, Triage, and Design; `format_spec` states that nobody will answer
  a question, so none is asked, and that an unknown is written as an
  `assumption` entry stating what is taken as true, after which the work
  continues on that basis; `parse` rejects a reply that asks anyway. Review
  and Implement are unchanged. Under `Checkpoint` every output is
  byte-identical to before.
- **A question that arrives anyway is a malformed reply.** The parser
  reports it as a transient agent error, counted against the card and
  retried until the card's own ceiling fails it, like any other reply that
  breaks the protocol. The dispatcher keeps blocking on a `Question` entry
  that reaches the board, which under BestEffort only a runner that
  bypasses the adapters can produce.
- **The agent authors the assumption.** It is the only party that knows
  what it assumed. Assumptions are not decisions: ADR-0005 is untouched,
  and a reviewer sees the reviewed card's assumptions under `UNDER REVIEW`
  and can send `ChangesRequested`.
- **Plumbing.** `CodexAgent` and `OllamaAgent` take the rule at
  construction through `with_stop_rule`, defaulting to `Checkpoint`;
  `orch_cli::allowed_kinds`, `format_spec`, and `parse` take a `StopRule`
  parameter; `orch_codex::output_schema(stop)` replaces the constant
  `OUTPUT_SCHEMA` and omits the `question` variant under BestEffort, so a
  schema-constrained model cannot form one. `rusty_orch` parses the goal
  file first and builds the adapters with its rule (`cli::run_with`; the
  four-argument `cli::run` keeps its signature and delegates). The text
  report lists live assumptions in their own block when any exist.
- **Not changed.** `orch-core`, `orch-dispatch`, the board rules, the
  blocking mechanism, and the binary's run loop.

This amends ADR-0004: `kind` is a function of the card's role and the
goal's stop rule. It resolves the consequence ADR-0009 recorded.

## Consequences

- A goal written with `"stop": "best_effort"` runs to the end without a
  human, and its report names every assumption the agents made.
- Signature changes within the family: `allowed_kinds`, `format_spec`,
  `parse`, both adapters' `render`, and `output_schema` for
  `OUTPUT_SCHEMA`. Every caller is in this family and updated here.
- The prompt is the one place model behaviour is steered; the two added
  rules are the whole of it. Whether small models follow them is an
  empirical question for the real-binary tests, not something the
  dispatcher can guarantee.
- Out of scope, by choice: an orchestrator-authored assumption, a
  non-blocking fallback in the dispatcher, promoting assumptions to
  decisions, and any per-card override of the goal's rule.
