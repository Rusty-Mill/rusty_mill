---
category: Added
changelog: Let Em Loose tab claims map, intent panel and stage strip view model in rusty_term (#1416 slice 2, not yet drawn).
---
## 2026-11-25 - LEL tab claims map view model

- **Added:** `gui/lel_map.rs` parses BBP `claim_card`/`claims_delta` entries into a claims map grouped by domain then dependency (Q40 interim), a read-only intent panel (version plus brief text) and the derived five-stage strip (LEL D189). Fixture-backed, no UI, no BBP change.
- **Known limitations:** The brief text has no source until the gated Mill Board read endpoints land (D173, Q64); Shadow run is never shown done (Q65).
