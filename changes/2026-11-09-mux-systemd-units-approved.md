---
category: Changed
changelog: The mux and rusty_term decision logs record Nano's go on C69 slice (c), systemd units and socket activation, as built in PR #1563.
---
## 2026-11-09 - mux systemd units approved

- **Changed:** `docs/projects/mux/DECISIONS.md` adds a dated row accepting C69 slice (c) as built in #1563 (user units, adoption of systemd's listener, close-on-exec, the NotFound-only "no server is running" message).
- **Changed:** `docs/projects/rusty_term/handoff/DECISIONS.md` adds D150 recording the same go.
- **Known limitations:** docs only; the code landed with PR #1563.
