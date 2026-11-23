---
category: Added
changelog: Mill Board web UI can close an unstarted card from its detail dialog; the dialog's dismiss button is now "Dismiss".
---
## 2026-11-18 - Mill Board: Close card in the card dialog

- **Added:** the card dialog has a **Close card** action (with a reason, confirmed once) that calls the same `card_close` as the MCP; it shows only for a role holding `triage` on a card in backlog or a queue. A started or blocked card says why it cannot be closed. Browser check: `tests/ui/close_card.mjs`.
- **Changed:** the dialog's old **Close** button, which only hid the dialog, is now **Dismiss**.
- **Known limitations:** the note field of `card_close` is not offered in the dialog; add a comment instead.
