---
category: Fixed
changelog: rusty_mux agent, role and card texts now refuse bidi overrides and zero-width characters, so a launcher cannot make a card id render reversed.
---
## 2026-11-18 - rusty_mux: AgentMeta refuses invisible formatting characters

- **Fixed:** `AgentMeta::is_valid` rejected only control characters (Unicode Cc). It now also rejects U+200B-200F, U+202A-202E, U+2060-2064, U+2066-2069 and U+FEFF. Review nit on #1800.
