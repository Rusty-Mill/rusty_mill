---
category: Added
changelog: Mill Board inbox Refine, one-allowance Approve on task totals, and Remediate cards for dropped acted-on work (#595).
---
## 2026-11-03 - Mill Board decision inbox: Refine, task totals, Remediate

- **Added:** Refine posts the human's clarification with `bbp human refine`, which refs the brief (C-R10). On a claim it then reopens the claim. On a paused task it then adds one allowance and resumes.
- **Added:** Approve on a task total adds exactly one allowance to each exhausted field (the limit the task opened with), then resumes the task.
- **Added:** a Reject, or a Choose, that drops acted-on work files one Backlog "Remediate" card in the owning card's project. The card lists each dropped claim with its action references (a PR, a deploy id) and BBP's dry run of the decision. It is tagged with the escalation id, so a retry finds the card instead of filing a second. `inbox_act` answers `remediated` with the card.
- **Added:** the Decisions view shows each crossed total, what Approve adds, and the work that was already acted on.
- **Known limitations:** needs the host follow-up (`bbp human refine` and the `brief`, `acted` and `totals` inbox fields). The Remediate card has no diffstat of the acted-on candidate.
