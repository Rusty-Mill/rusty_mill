# ADR-0008: Permanent agent errors

- **Status:** Accepted
- **Date:** 2026-10-02
- **Amends:** ADR-0007

## Context

ADR-0007 made deterministic refusals explicit through
`AgentRunner::supports`, which the dispatcher consults before counting a
call. That covers what is known before the call. Some refusals are only
discovered by making the call: Codex reports a missing login on stderr, a
delegating runner finds no adapter for the routed agent, the parser is handed
a role it does not serve. These came back as the single retryable
`AgentError`, so the dispatcher left the card `Running` and spent its whole
`max_calls` budget on calls that could not succeed.

ADR-0007 declined to type `AgentError` because it would make every error
construction choose retry policy. That cost is now accepted: the choice is
made once per construction site, in the adapter that knows the condition,
rather than by string matching in the dispatcher.

## Decision

- `AgentError` becomes an enum: `Transient(String)` and `Permanent(String)`.
  `message()` returns the reason, `is_permanent()` the classification,
  `Display` prints only the message. The tuple-struct constructor is gone;
  this is the breaking change the user approved.
- The dispatcher keys on the variant alone. A `Transient` error leaves the
  card `Running` as before. A `Permanent` error marks the card `Failed`
  with the error as the reason, then returns the same `DispatchError::Agent`.
  The failing call is still metered: it was made.
- Classification rule for adapters: `Permanent` only when no later call on
  the same card can clear the condition. Today that is a wrong agent or an
  unserved role at the adapter guard, Codex `not logged in`, and the parser
  refusing a role. Rate limits, process failures, timeouts, overflow, and
  malformed model output stay `Transient`.
- `FakeAgent` gains `Reply::Refuse` to script a permanent failure.

## Consequences

- A card hitting a permanent condition fails after one counted call instead
  of `max_calls`. Its dependents stay `Pending` and the loop reports `Stuck`,
  the same shape as retry exhaustion.
- Every `AgentRunner` implementation must choose a variant. Downstream code
  that constructed `AgentError(String)` or read `.0` must change to
  `AgentError::Transient(..)` / `AgentError::Permanent(..)` and `.message()`.
- No `orch-core` change. `DispatchError` keeps its shape; only the `Agent`
  variant's documented card state changes.
