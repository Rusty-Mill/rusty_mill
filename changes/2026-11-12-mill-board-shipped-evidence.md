---
category: Added
changelog: Mill Board: a Done card counts as shipped only with evidence (card_evidence); metrics report the rest as prototypes.
---
## 2026-11-12 - Mill Board: shipped needs evidence

- **Added:** `card_evidence` (needs `close`, `merge` or `triage`) adds up to 8 http(s) links showing a card's work shipped, such as the merged PR; `card_add` with `done: true` takes `evidence` too. Links live in the card's rusty_tick board document.
- **Changed:** `board_metrics` counts a Done card as shipped only with evidence; without, it is a `prototype` (per week and per project) and stays out of shipped and lead time. Observed external refs (ADR-0006) never count as evidence.
- **Known limitations:** every card Done before this change reads as a prototype until it gets a link. The roadmap's shipped lane and rollups still count every Done card that was not dropped. The card dialog doesn't show or add evidence yet.
