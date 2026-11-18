---
category: Added
changelog: Mill Board cards store four optional typed text fields, acceptance, scope, why and summary, each with a byte cap.
---
## 2026-11-18 - Typed card fields: schema and caps

- **Added:** cards carry `acceptance` (1,200 B), `scope` (800 B), `why` (600 B) and `summary` (320 B), stored in the card's rusty_tick board document and written through `Edit`. An over-cap write is refused with `Invalid` naming the field; an empty string clears a field. Empty fields are left out of the card JSON, so existing cards and payloads are unchanged. No rusty_tick change.
- **Known limitations:** the MCP parameters (#1580) and the UI (#1581) follow. The `summary` view omits all four fields: summary listings read no board documents, so `summary` on the card face needs a rusty_tick task field first (decision pending, #1568).
