---
category: Added
changelog: Ai Env Manager core crate (ai_env_core): entry model and link planner, pure, no dependencies.
---
## 2026-11-13 - Ai Env Manager core crate (M0 slice)

- **Added:** `crates/apps/ai_env_manager/crates/ai_env_core`, a pure, dependency-free crate with the entry model (skill, prompt, mod, agent, claude-md, agents-md), validated ids, and a planner (`plan_enable`, `plan_disable`) that never replaces or removes a path it cannot prove it owns. Overlap check recorded in the Ai Env Manager pack (D18).
- **Known limitations:** no tool catalog, filesystem linker, CLI or Windows CI yet; nothing touches the disk. Mill Board cards are not created.
