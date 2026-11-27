---
category: Added
changelog: Mill Board web UI gets a date filter (created, updated or moved, from/to) that survives a reload; card #1098.
---
## 2026-11-25 - Mill Board: date filter in the web UI

- **Added:** a compact date filter under the toolbar: a field (created, updated, moved) and a from/to day range. It uses the `created_*`, `updated_*` and `moved_*` arguments of `board_list` (#744), so `moved` matches the server's event log. The board keeps reading every card once; only the matching ids come back, and they are asked again whenever the board changes, here or in another session; a failure of an older request never undoes a newer answer. The filter is kept in the address (`?date=…&from=…&to=…`) and a chip clears it. `tests/ui/date_filter.mjs` checks it in a browser.
- **Known limitations:** `to` is the whole local day; with an active filter each poll makes one extra summary `board_list` call.
