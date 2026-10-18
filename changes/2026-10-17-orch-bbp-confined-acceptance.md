---
category: Added
changelog: Require Linux CI evidence for the confined Orch–BBP lifecycle and timeout, cancel and kill recovery.
---
## 2026-10-17 - Confined Orch–BBP acceptance

- **Added:** an unconditional Linux acceptance job feeding `required-gate`, using
  real Orch/BBP binaries, durable CLI resume, production sandbox supervision,
  independent scripted review, approval and recorded receipt.
- **Added:** separate strict confined timeout, cancellation and moderator-kill
  recovery cases, retaining running-workload and descendant cleanup assertions.
- **Validation:** each mode must execute exactly one test and emit completion
  evidence. Sandbox refusal, ignored/empty selections and missing evidence fail;
  per-mode logs, JSON results and a job summary are retained even on failure.
- **Known limitations:** deterministic MCP agents exercise the protocol, not live
  provider authentication or model behavior; no actual merge is performed.
