# ADR-0007: Explicit runner capabilities for permanent refusals

- **Status:** Accepted
- **Date:** 2026-10-02

## Context

Routing selects an `Agent` for every role, but a configured runner may not
serve every selected agent/role pair. In particular, the read-only Ollama and
Codex CLI adapters do not serve `Implement`. They previously returned the same
`AgentError` used for transient process and model failures. The dispatcher
therefore left the card `Running` and retried it until `TaskSpec::max_calls`
was exhausted.

Classifying `AgentError` text in the dispatcher would couple policy to adapter
wording. Making `AgentError` permanently/non-permanently typed would also make
every adapter error construction choose retry policy even though the permanent
condition is known before adapter setup.

## Decision

- Add the provided `AgentRunner::supports(agent, role)` method. Its default is
  `true`, so existing general-purpose and downstream implementations remain
  source compatible.
- Capability-limited adapters override the method. Ollama accepts `Local` and
  Codex accepts `Codex`; both accept Research, Design, Triage, and Review and
  reject Implement.
- After routing and before ledger checks or runner invocation, the dispatcher
  queries the capability. A pending card is started to record its selected
  agent and then immediately failed, without counting a call. A refusal returns
  the typed `DispatchError::UnsupportedRole` with that terminal reason.
  A later dispatcher run therefore cannot select it again, and its dependents
  remain pending.
- Composite/delegating runners delegate `supports` to the same child selected
  by `run`. Direct calls to each CLI adapter retain a guard before process or
  scratch-file setup.
- `AgentError` remains retryable. Its representation and the `orch-core` API
  and persisted plan/board shapes do not change. *Amended by ADR-0008:
  adapters may attach dispatcher policy through the additive `AgentFailure`
  result of `run_classified`; `AgentError` itself remains compatible.*

## Consequences

- Deterministically unsupported cards consume no goal or task calls and append
  no board entries. Codex creates no scratch files and neither adapter spawns a
  CLI for those cards.
- Transient failures retain the existing retry and ceiling behavior without
  error-string classification.
- The additive default cannot detect a capability omission in an old runner;
  capability-limited implementations must override it. This is preferable to
  silently breaking all existing `AgentRunner` implementations.
