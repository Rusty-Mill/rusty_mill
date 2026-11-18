---
category: Added
changelog: Mill Board web UI gains evidence, waits-on/related/superseded-by, Move to and card-creation details, and refused card writes are recorded in settings_audit.
---
## 2026-11-18 - Mill Board web UI parity with the MCP card actions

- **Added:** the card dialog adds evidence (`card_evidence`), sets waits on, related and superseded by (`card_plan`), and has a Move to select (`card_move`) beside drag. The add bar's Details takes a description, tags and a parent. The card face shows an evidence count. (#1574 to #1577, from the #1567 audit)
- **Added:** a refused or failed board write is appended to the settings audit file with the verified actor (what the call claimed is kept apart as `claimed_actor`), operation, card and a capped reason; a malformed argument is recorded as a fixed phrase, never its value, and the file stops growing at 4 MiB. `settings_audit` reads it back. (#1578)
- **Known limitations:** Move to lists every move from the column and the board refuses one the role may not make; there is no remove-evidence control because `card_evidence` has none; the audit file is capped, not rotated.
