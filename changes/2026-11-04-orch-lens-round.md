---
category: Added
changelog: "`orch-lens` runs one lens runner round (`round::round`): schedule the board's cards, call each job, post its answer through a `Board` port and record it."
---
## 2026-11-04 - orch-lens: one runner round

- **Added:** `round::round(config, view, state, ctx, models, board)` (Let Em Loose #594). It schedules the cards in the `View`, calls each job in turn, posts a non-empty answer as its lens through the new `Board` port and records it in the scheduler `State`. An answered review of a candidate also counts an attack pass for the fairness rule. A proposal slot no cleared provider can serve is settled.
- A provider's failure is recorded and the round goes on. The runner's own failures end the round and record nothing for that job, so the rules ask again: a task ceiling, an unsaved meter, a board that refused a post, or a lens with no prefix. An experiment stop only drops that job.
- **Changed:** `Job` carries the lens's `tools`, and `Models::get` takes them, so a read-only lens gets its read-only adapter. `State::abandon` drops a job without counting it.
- Pure over its ports; no shell or BBP transport yet. No new dependency.
