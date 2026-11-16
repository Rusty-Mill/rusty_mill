---
category: Added
changelog: Mill Board: a Done card counts as shipped only with evidence (card_evidence); metrics report the rest as prototypes.
---
## 2026-11-12 - Mill Board: shipped needs evidence

- **Added:** `card_evidence` adds up to 8 http(s) links showing a card's work shipped, such as the merged PR; `card_add` takes `evidence` too. Both need `close`, `merge` or `triage`. Links need a host and printable ASCII; a dropped card takes none. Links live in the card's rusty_tick board document.
- **Changed:** the Metrics view shows prototypes beside shipped work (a KPI and a Flow bar), and the MCP instructions and model guide tell agents to record the merged PR with `card_evidence`.
- **Changed:** `board_metrics` counts a Done card as shipped only with evidence; without, it is a `prototype` (per week and per project) and stays out of shipped and lead time. Observed external refs (ADR-0006) never count as evidence.
- **Added:** a one-off `--backfill-evidence REPO=OWNER/NAME,...` gives Done cards the pull requests their body and comments already name (dry run by default; `--backfill-record FILE` writes, `--undo-backfill FILE` reverts).
- **Known limitations:** a Done card that names no pull request reads as a prototype until it gets a link. The roadmap's shipped lane and rollups still count every Done card that was not dropped. The card dialog doesn't show or add evidence yet.
