---
category: Added
changelog: Mill Board enforces separation of duties (#447) - a per-card ledger of authors, reviews and gate approvals, an `approve` capability for the owner's voice proxy, and a Confirm tap for a merge or unblock the proxy filed.
---
## 2026-11-25 - Board separation of duties (#447)

- **Added:** `Card.duties` ledger (author, review, approval), kept whole in both stores, so no migration.
- **Added:** `approve` capability, `card_review` and `card_approve` ops, a `provider` on agents (inherited by minted workers), and `text_digest` in `card_get`.
- **Changed:** an author, or another agent of the same provider, may not review a card it built; a gate out of a work column needs a passing review since the last author claim. Admin holders (the owner) are exempt but recorded.
- **Changed:** the proxy crosses the design gates (Backlog to Ready, Architecting to Building) at once; a merge or an unblock is filed pending until an admin confirms it in the card dialog. A confirmed merge never moves the card.
- **Changed:** `card_approve` answers an `outcome` (`moved`, `recorded` or `expired`). An owner's tap on text edited since the filings expires them and moves nothing; the board logs it as the new `approval_expired` event and shows a toast.
- **Changed:** the instances of one credential (`cred/label`, `connector#id`) are one hand for authorship; each instance keeps its own verdict, at most four per credential on one column's work since the last authoring claim.
- **Known limitations:** BBP human-gate receipts (step 6) follow in a later change. Claims-mode verbs are unchanged (D3).
