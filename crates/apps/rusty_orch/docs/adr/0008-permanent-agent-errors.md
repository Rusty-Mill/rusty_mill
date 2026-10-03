# ADR-0008: Classified agent failures

- **Status:** Accepted
- **Date:** 2026-10-02
- **Amends:** ADR-0007

## Context

ADR-0007 made refusals known before a call explicit through
`AgentRunner::supports`. Some conditions are discovered only while invoking an
adapter. Truly permanent failures should not spend every retry, while a missing
Codex login must remain recoverable: a user can log in outside the process and
resume the same card. Charging each missing-login probe would disguise that
recoverable condition as terminal retry exhaustion.

`AgentError(String)` and its public `.0` field are already part of the runner
API. Changing it to an enum would break constructors, field access, and
exhaustive downstream assumptions solely to attach dispatcher policy.

## Decision

- Preserve `AgentError(String)` unchanged.
- Add `ClassifiedError`, with `Transient`, `Permanent`, and `Unavailable` variants,
  and an additive `AgentRunner::run_classified` method. Its default wraps the
  existing `run` result as `Transient`, preserving existing implementations.
- The dispatcher calls `run_classified`; it never classifies message text.
  Transient and permanent attempts are metered. Transient errors leave the card
  `Running`; permanent errors mark it `Failed`. An unavailable prerequisite
  leaves the card `Running`. The dispatcher preserves its check → start → count
  → run ordering, then rolls back exactly that invocation's goal and task
  charges (including removing a newly created zero-count task entry).
- Codex classifies `401`, `not logged in`, and login instructions as
  `Unavailable`. Repeated probes therefore cannot exhaust either call ceiling;
  after login, the same plan, card, board, and ledger can be passed to `run`
  again. Rate limits, process failures, timeouts, overflow, missing or malformed
  output remain ordinary transient failures.
- Capability guards remain the preferred permanent path when refusal is known
  before invocation. `FakeAgent` keeps its original two reply variants;
  dispatcher tests use a private typed runner for permanent and unavailable
  outcomes without expanding that public test API.

## Consequences

- Missing credentials produce an honest error on each explicit dispatcher run:
  there is no tight internal retry and no fabricated or partial output.
- An unavailable attempt does not consume `max_calls`; a later successful call
  does. Ordinary transient attempts still consume their budget and fail through
  the existing ceiling path. True permanent failures still fail immediately.
- The new enum and default trait method are additive. `AgentError(..)`, `.0`,
  existing runner implementations, `DispatchError::Agent`, and persisted
  `orch-core` shapes remain compatible.
