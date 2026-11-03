---
category: Added
changelog: Mill Board inbox Refine, one-allowance Approve on task totals, and Remediate cards for dropped acted-on work (#595).
---
## 2026-11-03 - Mill Board decision inbox: Refine, task totals, Remediate

- **Added:** Refine posts the human's clarification with `bbp human refine`, which refs the brief (C-R10). On a claim it then reopens the claim. On a paused task it then adds one allowance and resumes.
- **Added:** Approve on a task total adds exactly one allowance to each exhausted field (the limit the task opened with), then resumes the task. If an earlier Approve's extends committed but its resume did not, the host reports no exhausted field, and a retry only resumes; it adds no second allowance.
- **Added:** a Reject, or a Choose, that drops acted-on work files one Backlog "Remediate" card in the owning card's project. The card lists each dropped claim with its action references (a PR, a deploy id) and BBP's dry run of the decision. Its id is derived from the owning project and the exact decision (escalation id, the rev it was read at, and the action), so two different choices on one question are two records, and every other card id is random, so the card at that id is the record. Its `remediate:<escalation id>` tag is for people only: a card someone tags by hand, in this project or another, is not taken for the record and is never returned.
- **Added:** the card is written before BBP is asked to drop anything, so no drop sent from Mill Board can lose its card:
  - If the card cannot be written, nothing is sent.
  - If BBP refuses the drop outright, a comment on the card says nothing was dropped.
  - If BBP's answer is lost, the error names the card. A retry of the closed escalation answers `recorded` with that card and sends nothing to BBP again.
  - A successful decision answers `remediated` with the card.
- **Added:** decisions fail closed against a host older than #1420. Without its `acted` field a Reject or Choose that could drop a claim is refused, and without `totals` a task-total Approve or Refine is refused.
- **Added:** the Decisions view shows each crossed total, what Approve adds, and the work that was already acted on.
- **Known limitations:** needs the host follow-up (`bbp human refine` and the `brief`, `acted` and `totals` inbox fields). The Remediate card has no diffstat of the acted-on candidate.
