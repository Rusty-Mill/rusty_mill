---
category: Added
changelog: Mill Board lists open cards whose pull request already merged (`merged_open` in `board_metrics`), found by a read-only merge watch in the GitHub sync (card #1620).
---
## 2026-11-24 - Mill Board merge watch lists merged-but-open cards

- **Added:** each GitHub sync run reads the recently merged pull requests of every registered GitHub repository and lists the open cards they point at in `board_metrics.merged_open`: a `Closes card #N` line (`marker`), the card's own evidence (`evidence`) or a `card #N` mention (`mention`).
- **Added:** a pull request only points at cards of projects in the repository it was read from; cached matches are re-checked against the live card on every read; `board_metrics.merged_watch` reports when the list was refreshed, whether it was truncated and the last error.
- **Known limitations:** read-only; nothing is closed yet (marker auto-close and `card_unclose` follow in two PRs). It reads the 500 most recently updated closed pull requests per repository, not a date window. No UI badge yet.
