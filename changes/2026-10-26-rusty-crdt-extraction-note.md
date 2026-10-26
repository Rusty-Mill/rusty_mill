---
category: Changed
changelog: Design note for `rusty_crdt` (nexus-crdt reading, ADR-0002 check, revised API sketch); recommends parking the extraction.
---
## 2026-10-26 - rusty_crdt extraction: reading note and API sketch

- **Changed:** `docs/rusty-crdt-extraction-note.md` reads `nexus-crdt` in full, confirms that ADR-0002 Tier S allows a first-party workspace crate with no amendment, and revises the `rusty_crdt` API sketch. Fallible calls now return `Result`, and the sketch adds a freshness check and an id-reuse check that the earlier sketch lacked.
- **Known limitations:** docs only, nothing is built. The note recommends parking the crate: since #1052, `rusty_bbp`'s log-ordered line fold already matches RGA, and Nexus is frozen, so no live call site needs the library.
