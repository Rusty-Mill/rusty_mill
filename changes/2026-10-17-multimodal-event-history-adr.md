---
category: Added
changelog: Propose ADR-0138 for rusty_multimodal_db - how to support unbounded append-only event history (docs only).
---
## 2026-10-17 - ADR-0138: unbounded event history for the multimodal engine

- **Added:** `ADR-0138` (Proposed) compares extending `rusty_multimodal_db_engine` with a cold tier against keeping it as is and putting unbounded history in a consumer-owned segment log. It recommends the latter.
- **Known limitations:** Docs only; insert-log growth under sustained appends was not audited.
