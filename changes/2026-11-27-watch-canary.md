---
category: Added
changelog: `rusty_orch watch` re-runs each app's profile on its released ref through the new `bbp canary` (#599 PR 4, D217).
---
## 2026-11-27 - Canary on the released ref

- **Added:** `bbp canary --dir --task --repo-path --work --ref --profile` runs one profile of the task's frozen set on a clean sandboxed checkout of the resolved ref and prints the report as JSON. It writes nothing to the task.
- **Added:** `rusty_orch watch` gains a canary adapter. A red run posts one finding keyed `canary:<sha>:<profile>` (slot `canary:<profile>`), so a newer red release supersedes the older claim. A green run posts nothing.
- **Docs:** Let Em Loose D219 (Nano approved the remind-me route fix that #599 lessons need; Mill Board #1632).
- **Known limitations:** `watch_usd` isn't read yet, because no adapter calls a model. Deploying the daily watch on the BBP host is a separate step.
