---
category: Added
changelog: Approved design for Codex session resume in the BBP launchers (#432); build follows #600.
---
## 2026-11-26 - BBP Codex session resume design

- **Added:** `docs/projects/let-em-loose/432-codex-session-resume-design.md`, approved by Nano (D204): capture Codex's thread id from its first turn's `--json` log, store it per store/task/role, and resume with `codex exec resume <id>`, with the new launcher env var `BBP_PROVIDER_SESSION`. Capture defaults D203; Q66 resolved.
- **Known limitations:** design only; the build waits until the #600 builder work lands. The Codex CLI facts come from its docs and source and still need the S0 live check.
