---
category: Changed
changelog: Mill Board `board_list` pages serialize only their own cards; paging tests now cover rusty_tick with large records and writes mid-scan.
---
## 2026-11-10 - Mill Board list paging review fixes

- **Changed:** a paged `board_list` serializes only the page's cards, not the whole listing first.
- **Changed:** `TickStore::list` and `summaries` share one assembly.
- **Added:** tests over rusty_tick: a paged scan returns a 28 KB body, a 32 KiB prompt and a 31 KB comment whole; fenced writes and a new card between pages never repeat or drop a card. Order-dependent paging tests run on a ticking clock.
