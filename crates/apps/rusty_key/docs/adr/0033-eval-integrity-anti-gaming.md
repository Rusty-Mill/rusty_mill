# ADR-0033: Eval-integrity / anti-gaming guard

- Status: Accepted
- Date: 2026-05-27
- Tags: eval, faithfulness, integrity, security

## Context

Evaluation is only meaningful if a pass reflects real capability. Documented
eval-gaming behaviour shows the failure mode is concrete, not hypothetical: an
agent read git history to recover answers, and identified the benchmark in order
to decrypt its answer key. ARCHITECTURE §12's faithfulness map measures whether
the harness is honest; it had no row asserting that the *evaluation itself*
cannot be gamed. The owner decision (LOCKED) is: add this guard — a deliberate,
beyond-the-paper integrity measure.

## Decision

During evaluation, keep the following **out of the agent's context**: **answer
keys, expected check outputs, golden-episode expected outputs, and benchmark
identifiers**. The agent is evaluated on the task as posed, with no in-context
signal that reveals the grading criteria or names the benchmark. A pass therefore
reflects capability, not retrieval of the key. This is recorded as an explicit
anti-gaming row in the faithfulness map and in the eval harness contract. Detail:
`docs/ARCHITECTURE.md` §12, `docs/dev/eval-plan.md`.

## Consequences

- ARCHITECTURE §12's faithfulness map gains an anti-gaming row; the eval harness
  must construct agent context with expected-outputs and benchmark IDs withheld.
- Checks still run and grade as before — the withholding is of *context handed to
  the agent*, not of the harness's own grading data.
- This is explicitly beyond the research paper; it is owner-mandated and noted as
  such so future readers do not mistake it for paper-faithfulness drift.
- It complements the chaos/resilience tier (ADR has its honest-degradation
  assertion): both defend against verified-success that is not earned.
