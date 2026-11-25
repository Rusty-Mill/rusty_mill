---
category: Fixed
changelog: Mill Board `card_claim` and `card_move` now hold the link gate, so a card cannot start under a concurrent `card_unclose`; card #1620 follow-up.
---
## 2026-11-25 - Mill Board: claims and moves hold the link gate

- **Fixed:** `card_unclose` refuses while an open dependent has started, but a claim or a move into a work column could start the dependent between its scan and its write. `Board::claim` and `Board::move_card` now take the link gate (before the workflow guard, the fixed lock order) that takes and dependency edits already held. Deterministic tests hold a claim and a move mid-commit and show the unclose waits and then refuses.
- **Known limitations:** the gate is per board process; a second board on the same store is not covered. `auto_close` stays off by default (ADR-0008).
