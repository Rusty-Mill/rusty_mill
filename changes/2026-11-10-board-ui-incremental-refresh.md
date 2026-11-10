---
category: Changed
changelog: Mill Board's web UI re-reads only the cards that changed, not the whole board, after each poll and each of its own writes.
---
## 2026-11-10 - Mill Board web UI incremental refresh

- **Changed:** after the first read, the web UI reads only the cards named by new board events, one `card_get` each, whether another actor or the page itself changed them. An edit now costs about 2 KB instead of the whole board (about 2 MB on a production-sized board). A card that left the board or the viewer's scope leaves the page.
- **Changed:** the UI reads the whole board again only on a `resync` (the board restarted) or when more than 50 cards changed between two polls.
- **Known limitations:** the first read when the page opens is still the whole board.
