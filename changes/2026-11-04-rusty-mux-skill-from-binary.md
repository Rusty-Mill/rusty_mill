---
category: Changed
changelog: rusty-mux-ctl agent setup installs a stub skill that reads its guide from `rusty-mux-ctl agent skill`, so the guide always matches the installed ctl.
---
## 2026-11-04 - rusty_mux skill read from the binary (C73)

- **Added:** `rusty-mux-ctl agent skill` prints the agent guide built into ctl.
- **Changed:** the `SKILL.md` that `agent setup` installs is now a stub that runs
  `agent skill`; if ctl is not on PATH it tells the agent to say so, not to guess. A skill
  installed by an older ctl shows `outdated` in `agent status`; rerun `agent setup` once.
