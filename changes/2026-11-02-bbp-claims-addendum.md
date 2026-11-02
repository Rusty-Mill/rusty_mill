---
category: Added
changelog: BBP claims addendum (draft): claims mode with turnless evidence, board-computed scores and escalation; spec only.
---
## 2026-11-02 - BBP claims addendum (draft)

- **Added:** `rusty_bbp/docs/spec/bbp-claims-addendum.md`, a draft spec for claims mode (Let Em Loose, card #593). Agents post claims and evidence with no turn; the board scores claims with an integer rule and accepts, drops, voids or escalates them. It sits beside the stage lifecycle and is chosen per task. The spec includes 28 trace tests to write before the engine change.
- **Known limitations:** spec only, with no engine change. Log schema 5, two new message kinds and the session credential need owner approval before implementation. Five open questions remain.
