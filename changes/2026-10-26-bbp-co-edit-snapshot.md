---
category: Added
changelog: BBP co-edited spec versions carry a snapshot (SHA-256 of their text), shown on the human view of an open plan gate.
---
## 2026-10-26 - BBP co-edited spec snapshots

- **Added:** each co-edited spec version carries `snapshot`, the SHA-256 of its materialized text, derived on fold (no log format change). Equal text gives an equal snapshot.
- **Added:** `TaskView.proposal_snapshot` names the snapshot of the co-edited version an open plan gate proposes, so a human approval can be checked against the bytes read. Omitted from JSON when absent.
- **Known limitations:** the approval itself still binds by version id; it does not carry the hash.
