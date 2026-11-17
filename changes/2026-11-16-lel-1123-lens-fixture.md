---
category: Added
changelog: Let Em Loose v0 lens config and four lens prompts as an orch-lens test fixture (#1123), no live calls.
---
## 2026-11-16 - Let Em Loose v0 lens prompts and config fixture

- **Added:** `orch-lens/tests/fixtures/lens/` with `lens.json` and the architect, research, adversary and operator prompts, and `tests/lens_fixture.rs`, which parses the config and reads each prompt (`lens check` never does).
- **Known limitations:** the llama-swap port and Claude/Codex pins are unverified; Factory remains the source of truth (a copy of these files goes there).
