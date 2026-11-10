---
category: Changed
changelog: The mux and rusty_term decision logs record Nano's go on the C36 (b) clean stop (#1589) and the C33 force-disconnect restriction (#1590) as built.
---
## 2026-11-10 - mux clean stop and force-disconnect restriction approved

- **Changed:** `docs/projects/mux/DECISIONS.md` adds dated rows accepting C36 slice (b) as built in #1589 (client message `Shutdown` 0x24, `Features::SHUTDOWN` bit 38, `rusty-mux-ctl server stop`) and the C33 force-disconnect restriction as built in #1590. The two Proposed rows above them stay as the record of what was proposed.
- **Changed:** `docs/projects/rusty_term/handoff/DECISIONS.md` records the same go.
- **Known limitations:** docs only; the code landed with PRs #1589 and #1590.
