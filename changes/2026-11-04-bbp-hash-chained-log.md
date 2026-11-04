---
category: Added
changelog: BBP task logs are hash-chained; every load checks the chain and `bbp verify` prints its head (#722).
---
## 2026-11-04 - Hash-chained BBP task logs

- **Added:** each line of a task log now carries `prev`, the SHA-256 of the line before it (or of the whole unchained prefix for an existing log's first chained line). Every load checks the chain and refuses an edited, removed, reordered or inserted line as corrupt.
- **Added:** `bbp verify --dir D --task T [--expect H]` checks one log under the lock without changing it and prints `lines`, `unchained`, `head` and `partial`; `--expect` passes when `H` is the hash of any line, so a saved head still verifies after appends.
- **Known limitations:** an older build reads a chained log as corrupt, so do not roll a data directory back to one. Someone who can write the log can rewrite it and recompute every hash; anchoring the head outside the host is a follow-up. No schema bump (D69).
