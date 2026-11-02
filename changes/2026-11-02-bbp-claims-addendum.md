---
category: Added
changelog: BBP claims addendum: claims mode with turnless evidence, board-computed scores and escalation; spec only.
---
## 2026-11-02 - BBP claims addendum

- **Added:** `rusty_bbp/docs/spec/bbp-claims-addendum.md`, the spec for claims mode, approved by the owner 2026-11-02 (Let Em Loose, card #593). Agents post claims and evidence with no turn; the board scores claims with an integer rule and accepts, drops, voids or escalates them. It sits beside the stage lifecycle and is chosen per task. The spec includes 35 trace tests to write before the engine change.
- **Known limitations:** spec only; the engine change is a separate PR. Open questions Q1 to Q5 stand at their proposals.
