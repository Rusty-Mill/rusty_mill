---
category: Added
changelog: The Mill Board web UI shows a card's summary on its face and lets people see and edit its acceptance, scope, why and summary with byte counters.
---
## 2026-11-19 - Typed card fields: UI (#1581)

- **Added:** a card's `summary` shows under its title on the board, roadmap and archive; a card with none shows nothing extra.
- **Added:** the card dialog has Summary, Acceptance and Scope boxes and a collapsed Why section, each with a live byte counter (320 / 1,200 / 800 / 600) that turns red past the cap. Save refuses an over-cap field with a message and sends nothing; an emptied field is cleared, and a Save sends only the typed fields that changed, so untouched text (line breaks, CRLF) is never rewritten.
- **Added:** `tests/ui/typed_fields.mjs`, a browser check of show, edit, byte counting, the over-cap refusal and empty fields.
- **Known limitations:** the page already loads full cards, so Why is hidden, not fetched on request. Reading summaries only is the card view's own change (the summary view omits `summary` until the tick adapter is removed). The migration of headed sections is #1582.
