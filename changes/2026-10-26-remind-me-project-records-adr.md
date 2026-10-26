---
category: Added
changelog: remind-me ADR-0027 (proposed): projects as entities with evidenced stage, status and anchor claims.
---
## 2026-10-26 - Proposed design for first-class project records in remind-me

- **Added:** ADR-0027, status Proposed. A project is an entity with a live `project.stage` claim; stage, status and anchors (repo, path, tracker, parent) are SPO fact memories that must cite evidence; proposals are computed from captured context and only a person confirms them. No schema, sync-contract or migration change.
- **Known limitations:** design only; nothing is implemented until the owner approves.
