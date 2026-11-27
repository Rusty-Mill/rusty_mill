---
category: Added
changelog: Design for anchoring the BBP claim-log head on the Mill Board, outside the BBP host user (#1551).
---
## 2026-11-27 - Design: anchor the BBP claim-log head outside the host (#1551)

- **Added:** `docs/projects/let-em-loose/1551-claim-log-anchor-design.md`. The Mill Board, running as its own user, pulls each linked task's head with `bbp verify --expect <last anchor>` and records it as an append-only card comment, so a full rewrite of the log by `orch-runner` is caught. No BBP log format or API change.
- **Known limitations:** design only, nothing is built. Root on the host is not covered without an off-machine copy (open question).
