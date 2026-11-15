---
category: Changed
changelog: Ai Env Manager records Nano's answer that Windows skill edits stay in-place, not crash-safe (D39), and that Unix saves do not carry extended ACLs (D40).
---
## 2026-11-15 - Ai Env Manager: Windows edits in place, Unix ACL limit (D39, D40)

- **Changed:** D39 records Nano's answer on a decision card: on Windows, skill edits are written in place and are not crash-safe ("Leave as is"). A failed save attempts best-effort restoration of the old text. Unix keeps owner, group and mode through an atomic save.
- **Changed:** D40 records Nano's answer to accept the POSIX ACL limit: Unix saves stay atomic and keep owner, group and mode, but extended ACLs are not carried over.
- **Known limitations:** Docs only; no code changes. On Windows, damage is possible on a crash or a failed rollback.
