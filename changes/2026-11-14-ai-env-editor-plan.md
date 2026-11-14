---
category: Changed
changelog: Ai Env Manager design pack puts in-app file editing in GUI v1 (D37) and adds a design plan for the editor.
---
## 2026-11-14 - Ai Env Manager: in-app editing goes into GUI v1

- **Changed:** Nano answered Q14 ("I'd like it in"), so in-app editing of cataloged files joins GUI v1 (D37, widens D34; order in D33/D35 unchanged). Added `docs/projects/ai-env-manager/editor-plan.md`: editable files, plan-then-confirm rule, backend commands in `ai_env_app`, CSP limits, edge cases and slice order.
- **Known limitations:** docs only. Which file kinds and what "edit" covers are open as Q15; the plan assumes skill files and text edits to existing files. Stacked on #1703 (D35, D36, Q14).
