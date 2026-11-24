---
category: Changed
changelog: Let Em Loose tab in rusty_term has a written slice plan (docs only).
---
## 2026-11-24 - LEL tab slice plan (#1416)

- **Changed:** `docs/projects/let-em-loose/1416-lel-tab-slice-plan.md` orders the build into six slices and records the data-source decision (Mill Board HTTP API, D173). No code.
- **Known limitations:** claims, intent and spend need new Mill Board read endpoints (a public API change with its own gate); until then those slices use fixtures. Nano must approve the plan (D25) before slice 1.
