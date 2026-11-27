---
category: Added
changelog: Design for anchoring the BBP claim-log head on the Mill Board, outside the account that writes the log (#1551).
---
## 2026-11-27 - Design: anchor the BBP claim-log head outside the log writer's account (#1551)

- **Added:** `docs/projects/let-em-loose/1551-claim-log-anchor-design.md`. The Mill Board, running on the same host as its own user, pulls each linked task's head with `bbp verify --expect <last anchor>` and records it as an append-only card comment. A rewrite by `orch-runner` of the log up to the last anchor is caught, including any legacy prefix the chain covers. No BBP log format or API change.
- **Known limitations:** design only, nothing is built. The design needs Nano's approval, and the host changes (guard, timer) need his separate host-change authorisation. Root on the host is not covered: an off-machine copy is deferred (D215) until outside or untrusted agents join. A log made only of legacy unchained lines is not anchored.
