---
category: Security
changelog: Mill Board gets a narrow `record` capability for recording merged work, so agent roles no longer need `close`.
---
## 2026-11-13 - Mill Board: record merged work without close

- **Added:** capability `record` (Millwright G1, M1). Like `close`, it moves a claimed Building or V&Ving card to Done and lets `card_add` with `done: true` record merged work, but only when the card or call carries a GitHub pull request link as evidence. It also adds evidence. It cannot archive or restore. `card_evidence` and `evidence` on `card_add` accept it.
- **Changed:** the capability list in settings gains `record` (11 capabilities); existing roles are unchanged.
- **Known limitations:** the link is checked for shape (`https://github.com/<owner>/<repo>/pull/<n>`), not asked of GitHub whether it merged. That forge check is the next step.
