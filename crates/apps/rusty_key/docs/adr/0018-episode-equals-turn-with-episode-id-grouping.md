# ADR-0018: Episode = turn, with `episode_id` grouping

- Status: Accepted
- Date: 2026-05-27
- Tags: faithfulness, eval, observe

## Context

The AI Harness Engineering paper treats the whole task as its unit of evaluation
(one episode per task). Rusty Keys instead emits one episode package per `send()`
turn — its unit is the turn, not the task (consolidated plan §G). This is a
deliberate faithfulness divergence: a turn-grained package is cheaper and matches
the `Session::send()` boundary, but it makes RK's "episode" a narrower object
than the paper's, which complicates task-level metric comparison.

## Decision

Keep episode = turn (one package per `send()`), and add an `episode_id` field so
the turns belonging to one task can be grouped back into a task-level episode for
evaluation. The package schema and `episode_id` semantics live in
`docs/architecture/data-model.md`; the faithfulness map is in
`docs/ARCHITECTURE.md`.

## Consequences

- Turn-level packages stay cheap and aligned with the `send()` boundary.
- Task-level metrics must aggregate over `episode_id` rather than reading a single
  package.
- The paper's unit "task" is confirmed against the clean extraction (p.8: "the
  unit of evaluation is the episode, not a single model response"); the PDF
  verification caveat is therefore lifted. Turn-as-stand-in-for-task with lossless
  `episode_id` regrouping is the ratified faithfulness divergence (Round 3 D1).
- M-HIR is computed over `episode_id`: the metric family aggregates turn-grained
  packages back to the task unit, so the divergence costs no comparability.
