---
category: Added
changelog: "`bbp erosion approve` records Nano's approved erosion limits, and an enforcing claims open must use exactly those limits (D115)."
---
## 2026-11-06 - Erosion gate: approved limits

- **Added:** `bbp erosion approve --dir D --concentration-bp N --duplication-bp M --source REF [--as WHO]` and `bbp erosion limits --dir D` (card #704, D115, D118). An approval is one JSON line in the append-only `<dir>/erosion/limits.log`, written under its lock, and the newest line wins. `--source` names where Nano approved the limits; the host records it and doesn't verify it.
- **Changed:** a claims open whose erosion profile enforces limits is refused `bad_claims_config(erosion)` when no limits are recorded, when its limits differ from the newest record, or when the log is damaged. Report-only, `--cleanup` and `"off"` opens are unchanged.
- **Docs:** D118.
