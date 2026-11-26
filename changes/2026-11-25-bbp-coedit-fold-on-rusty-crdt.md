---
category: Changed
changelog: rusty_bbp's co-edited spec fold now runs on rusty_crdt's RGA; admission and logs are unchanged.
---
## 2026-11-25 - rusty_bbp co-edit fold on rusty_crdt

- **Changed:** `rusty_bbp::coedit::Doc` keeps its lines in `rusty_crdt`'s `Rga` instead of its own spliced vector (#430, D176). `rusty_crdt` has its first user. The public API, admission (the overlap refusal), the log and the golden logs are unchanged; a differential test checks the new fold against the old one on edge cases and random histories.
- **Added:** `rusty_crdt::Rga::elements`, every element with its visibility, tombstones included, in sequence order.
- **Known limitations:** an insert that reuses a line id in `Doc::apply` (a reused version id, even with identical text) now fails with that line id, where the old fold silently held two lines with one id. The engine never reuses one. Deleting a tombstone again still succeeds, and an insert with no lines still fails on a missing anchor, as before.
