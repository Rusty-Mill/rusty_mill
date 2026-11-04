---
category: Added
changelog: "BBP claim cards carry each claim's `accept_at` line and every evidence item's `strength` and `note`, so `rusty_orch lens run` now starts."
---
## 2026-11-04 - BBP: accept line and evidence notes on the claim card

- **Added:** `ClaimView.accept_at` on `claim_card` and `claims_delta` entries (Let Em Loose #594, D56, approved by the owner 2026-11-04). It is the claim's own line from the task's frozen config: `accept_at_strict` for high stakes or an irreversible claim, otherwise `accept_at`. `EvidenceView` gains `strength` and `note`, as posted (closes Q12 and Q13). These are views only: nothing is written to the log, and the schema and protocol versions are unchanged. Addendum item 13 and trace tests CL-102 and CL-103.
- **Changed:** `rusty_orch lens run` now starts. The lens runner takes each claim's accept line and evidence notes from the card, so it no longer needs the task's claims settings (the `UNKNOWN_CLAIMS` refusal is gone) and no longer keeps notes from `read` pages. `Card::from_view` takes the view, the claim body and the lens config. A host without the new fields fails to decode the card, so the runner never guesses a line.
