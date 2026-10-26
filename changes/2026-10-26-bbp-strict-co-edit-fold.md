---
category: Fixed
changelog: BBP records a co-edited spec version that does not apply on replay instead of skipping it, and refuses edits or gates on it by name.
---
## 2026-10-26 - BBP strict co-edit fold

- **Fixed:** folding a log whose co-edited spec version names a missing line, or a base that is not an indexed version, records it in `TaskState::unapplied` with what it lacked. Before, the version was silently skipped, and one with an unknown base was folded as the start of a new spec. An edit or plan-gate request on such a version is refused `ref_unresolved` with the missing line or base named.
- **Known limitations:** the engine never writes such a log; this guards replay of foreign or damaged logs only.
