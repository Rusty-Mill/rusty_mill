# ADR-0021: The lens runner's pure core

- **Status:** Accepted (Let Em Loose #594 design rev 3, with #598/#599 answers)
- **Date:** 2026-11-03

## Decision

`orch-lens` holds everything the lens runner decides, with no I/O: the
`Model` port, the reply protocol, configuration checks, context assembly,
data classes, tiering, the money meter, the scheduler and track records.
The `rusty_orch lens` shell (a later PR) only reads the board, calls
providers and posts.

- **Rules are checks.** Blind first, look harder once, two proposers plus
  a rebel before any attack, no jobs on a sole candidate, fair attacks and
  "agreement costs evidence" are enforced by the scheduler and by `parse`,
  not by prompt text.
- **Data classes replace `sensitive_domains`** (#598 2.2). Providers declare
  `max_class`; CLI (cloud) providers default to and cannot exceed
  `internal`, the HTTP adapter (local only) defaults to `sensitive`. Items
  above a provider's clearance never enter its context, and a claim above
  every clearance gets no job.
- **Money is integer micro-dollars** with reserve-before-call, so parallel
  jobs cannot pass a ceiling. Every provider needs a `price` block (0 is
  allowed, missing is not). The experiment ceiling stops paid calls only;
  task spend and call ceilings stop the task. One meter holds a whole
  experiment, every task's settled and reserved spend, so tasks cannot
  overspend the experiment together; reservations bound the worst case
  (a token per prompt byte plus framing, at the dearer input rate).
- **A question** is an open claim with the domain `question` and no
  `answers`. BBP has no question flag, so orch reads it from the domain the
  intent gives it.
- The reply protocol is one JSON object `{"posts": [...]}` with the fields
  of the host's `post` tool, strictly checked.

## Consequences

- The track record is informational only (#599 Q2).
- Persisting runner state and reservations, the breaker (observe-only,
  #598 Q4), lessons (#599) and the `task_paused` host write wait for the
  shell and the #598 host changes.
