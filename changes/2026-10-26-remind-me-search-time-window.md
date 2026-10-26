---
category: Added
changelog: remind_me_search takes `since`/`until` (RFC 3339 or ages like 2d) and `time_field`; an empty query with a window lists it newest-first.
---
## 2026-10-26 - remind_me_search time window

- **Added:** `since` and `until` on `remind_me_search`, each RFC 3339 or a relative age (`30m`, `12h`, `2d`, `1w`), over `created_at` by default or `updated_at` with `time_field: "updated"`. The window is half-open (`since` inclusive, `until` exclusive) and is applied before the keyword limit, so pages are not short.
- **Added:** an empty `query` with a bounded window lists that window newest-first, so "what happened in the last two days" is one call. An empty query with no window still returns nothing.
- **Validation:** a bad format, a zero or negative age, or `until` at or before `since` is rejected with the argument named.
- **Known limitations:** on the semantic half the window is a post-filter, as the project/branch scope already is, so a narrow window can leave fewer semantic candidates; the keyword half is unaffected.
