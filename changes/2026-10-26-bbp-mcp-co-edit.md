---
category: Added
changelog: bbp mcp lets agents write co-edited spec versions by line id and read them with their lines, head and snapshot.
---
## 2026-10-26 - Co-edited specs through bbp mcp

- **Added:** `put_artifact` kind `spec` takes `edit` (`base`, and `ops` of `insert {after, lines}` or `delete`, line ids written `art:N#M`) in place of `body`, storing a co-edited version. Malformed edits are tool errors; the core still validates and admits them.
- **Added:** `get_artifact` on a co-edited version returns its text as `content`, plus `lines` (each with its id), `head` (the spec's latest version) and `snapshot`.
- **Known limitations:** the shipped planner prompts do not ask for co-editing yet; a live N=4 run still needs owner approval for spend.
