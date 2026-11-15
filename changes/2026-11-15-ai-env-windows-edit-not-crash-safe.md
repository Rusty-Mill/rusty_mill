---
category: Changed
changelog: Ai Env Manager records Nano's answer that Windows skill edits stay in-place, not crash-safe (D39).
---
## 2026-11-15 - Ai Env Manager: Windows skill edits stay in place (D39)

- **Changed:** D39 records Nano's answer on a decision card: on Windows, skill edits are written in place and are not crash-safe. Unix keeps owner, group and mode through an atomic save.
- **Known limitations:** Docs only; no code changes. A crash mid-write on Windows can leave a partial file.
