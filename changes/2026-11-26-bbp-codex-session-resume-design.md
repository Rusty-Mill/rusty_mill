---
category: Added
changelog: Design for Codex session resume in the BBP launchers (#432), awaiting approval.
---
## 2026-11-26 - BBP Codex session resume design

- **Added:** `docs/projects/let-em-loose/432-codex-session-resume-design.md`: capture Codex's thread id from its first turn's `--json` log, store it per store/task/role, and resume with `codex exec resume <id>`. Decision D199 and open question Q66.
- **Known limitations:** design only; the Codex CLI facts come from its docs and source and still need the S0 live check.
