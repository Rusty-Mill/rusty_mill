---
category: Added
changelog: "`erosion check --cleanup` enforces D44's cleanup predicate (neither share rises, one falls), and claims profile sets may use it."
---
## 2026-11-05 - Erosion gate: cleanup mode

- **Added:** `erosion check --base C --cleanup` (card #704 slice 4a, D44, D113). It passes only when neither share rises and at least one falls, so an unchanged tree fails where 0 / 0 limits would pass it. The line's `mode` is `"cleanup"` and `within_limits` is whether the predicate held. A measure error fails closed (exit 2), as with limits.
- **Changed:** `rusty_bbp_host` accepts `--cleanup` as a third erosion profile mode. It is not report-only, so the D88 ledger doesn't count it. The inbox detail's `mode` reads `"cleanup"` as a named mode.
- **Docs:** D113 (slice 4 build defaults) and OPEN_QUESTIONS Q37 (which holdouts a cleanup keeps green).
