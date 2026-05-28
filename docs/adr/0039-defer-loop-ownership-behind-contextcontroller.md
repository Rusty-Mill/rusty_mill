# ADR-0039: Defer loop ownership; revisit behind a `ContextController` seam

- Status: Proposed
- Date: 2026-05-28
- Tags: kernel, constrain, loop, context, roadmap

## Context

The think-act-observe loop is delegated to aisdk; the kernel recovers policy
control by intercepting the per-tool `execute` closure (ADR-0024,
`crates/kernel/src/lib.rs`). This already buys the two things loop-ownership
would: policy vetting (inside `ToolDispatch`) and a structured `ToolOutcome`
(returned so aisdk cannot re-stringify a block as a generic error).

The benchmarked Claude Code writeups own their loop and run work *between*
model steps — most importantly mid-loop compaction. In RK, compaction runs once
per `Session::send` *before* `run_turn` (`crates/app/src/session.rs`), never
between a turn's tool calls; a single turn emitting many large tool outputs can
grow the window with no compaction opportunity. The harness-assessment review
(`docs/assessment/RECOMMENDATIONS.md`) examined wrap-vs-own and concluded the
cost of owning (re-implementing aisdk's message threading and provider-side
retry/backoff) is not yet justified.

## Decision

**Keep delegating the loop to aisdk for now.** Do not own the loop until a
concrete *between-steps* requirement lands — specifically any of: mid-loop
compaction, per-step policy escalation/interruption (cancel after step N on a
budget breach), or step-level tracing fidelity beyond the current post-turn
`final_reached` flag.

When that trigger arrives, own the loop **only behind a new abstract seam**:
define a **`ContextController` (`async fn between_steps(&mut history) -> Action`)
trait in `constrain`**, implemented by `feed`/`app` (the compactor) and called
by an owned kernel loop. This preserves the kernel invariant — it still imports
neither `feed` nor `compose`, only the `constrain` trait. The owned loop is a
drop-in alternative `run_turn`: `Session` already calls only
`run_turn(model, system, prompt, dispatch)`, and `FakeLanguageModel` implements
the real aisdk `LanguageModel` trait, so the seam is testable today.

As an interim guard until then, wire `max_steps` via `stop_when` (the harness
P0 safety-floor item) and add tool-output truncation in `feed` to bound
within-turn window growth.

## Consequences

- The kernel stays genuinely thin and the DAG invariant is trivially preserved
  while no between-steps requirement exists.
- A clear, abstract trigger and seam are recorded, so owning the loop later is a
  scoped change rather than a rewrite.
- The interim `max_steps` + output-truncation guard mitigates the
  within-turn-overflow risk without taking on aisdk's retry/threading surface.
- When loop ownership is taken, this ADR supersedes the Strategy-A rationale in
  ADR-0024 for the loop body (the tool-seam interception remains valid for the
  delegated path).
