---
category: Changed
changelog: rusty_remind_me: search ranks facts and summaries first and leaves raw session dialogs out unless include_raw, a dialog category or a session filter asks for them; a search nothing refined matches falls back to marked dialogs (ADR-0029).
---
## 2026-10-26 - remind-me: search ranks refined memories first

- **Changed:** `remind_me_search`, the CLI search and the hook context no longer rank `dialog` and `skeleton` memories by default. A captured dialog runs to 200,000 characters and matched almost any query, crowding out the facts decomposed from it. The facts carry `source_capture_id`, which leads back to the dialog.
- **Added:** `include_raw` (default false) on `remind_me_search`. A `category` of `dialog` or `skeleton`, or a `session_id` filter, turns it on implicitly.
- **Added:** when nothing refined matches, the best raw dialogs are returned and marked `raw_fallback: true`, and the Markdown reply labels them as raw sessions. A session that has not been decomposed yet stays findable.
- **Known limitations:** this is a behaviour change for existing callers, and they can opt back in per call. The API's paged `GET /api/memories/search` is unchanged.
