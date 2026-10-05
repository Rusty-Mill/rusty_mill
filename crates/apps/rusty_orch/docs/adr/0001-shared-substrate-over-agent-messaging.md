# ADR-0001: Shared substrate over agent messaging

- **Status:** Accepted
- **Date:** 2026-10-01

## Context
Multiple models (Claude, Codex, Gemini, local Ollama/Hermes) should collaborate without a human copying context between them. Naive multi-agent designs pass natural-language messages; every hand-off is serialized, re-ingested, and summarized again, so the same information is paid for repeatedly and degrades with each hop. Claude, ChatGPT, and Gemini are used through subscriptions, not API keys.

## Decision
- Agents share state through a per-goal append-only blackboard plus the git repo, and exchange only references.
- A deterministic dispatcher (not an LLM) routes task cards by role. Cards never name an agent.
- Each vendor is reached only through its official CLI in headless mode, on its own subscription login.
- Every task's output is reviewed by a different agent than its author.

## Consequences
- Token cost scales with what each agent actually reads, not with conversation length.
- Invariants (no self-review, no dangling refs, acyclic plans) live in a pure core and are unit-tested.
- Subscription rate limits, especially Claude's, become the main throughput constraint; heavy work routes to Codex and local models.
- Dependence on CLI headless behavior: each adapter must be verified per vendor before use.

**Update (2026-10):** the Gemini CLI is discontinued, so no Gemini adapter will be built. `Agent::Gemini` remains a routing label in `orch-core` and in persisted records; nothing serves it.
