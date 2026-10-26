---
category: Added
changelog: remind-me ADR-0027 (accepted): projects as entities with evidenced stage, status and anchor claims.
---
## 2026-10-26 - Design for first-class project records in remind-me

- **Added:** ADR-0027, accepted by the owner. A project is an entity with a live `project.stage` claim; stage, status and anchors (repo, path, tracker, parent) are SPO fact memories that must cite evidence; proposals are computed from captured context and only a person confirms them. No schema, sync-contract or migration change.
- **Known limitations:** design only; implementation follows in three slices.
