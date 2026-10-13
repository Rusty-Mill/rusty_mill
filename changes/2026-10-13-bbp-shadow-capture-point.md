---
category: Changed
changelog: **BBP event-driven note: shadow comparison is captured at `schedule()`.** Explicit grants are labelled and keep the forced grantee as their counterfactual, and the capability filter now requires current-stage `verdict_at` for verdict posts.
---
## 2026-10-13 - BBP event-driven note: shadow capture point

- **Changed:** shadow decisions are captured in `schedule()` / `pick_next`, not in `grant()`, so yield, human `ask`, settlement and consultation returns are not counted as disagreements. Added planned comparison tests for those paths and aligned the capability-filter text with `check_verdict` (R6).
- **Known limitations:** design only.
