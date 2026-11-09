---
category: Fixed
changelog: Bound Mill Board's rusty_tick collection reads so board growth cannot exceed the HTTP response buffer.
---
## 2026-11-09 - Bound Mill Board storage transfers

- **Fixed:** Mill Board consumes byte-bounded, UUID-cursor pages for rusty_tick tasks and documents; individual UUID card reads retrieve their own document and comments. Existing MCP responses and stored records stay compatible, with no data migration or raised HTTP limit.
- **Known limitations:** Complete board results still materialize all returned cards; deploy rusty_tick before Mill Board. See the storage response-size incident runbook for measurements, integrity audit and rollback.
