---
category: Added
changelog: rusty-mux-ctl archive-block saves a finished block's transcript and outcome, with archive-list and archive-show to read them.
---
## 2026-11-24 - Archive of dismissed blocks, slice 1 (C84, card #1570)

- **Added:** `archive-block` saves a block whose program is gone (screen, history, exit status, card, label, why, who) under `<state>/archive/` and closes it; `archive-list` and `archive-show` read the records back. New `Features::ARCHIVE` (bit 54) and messages 0x31 to 0x33. A record whose text looks like a secret is marked `sensitive`; nothing is redacted.
- **Known limitations:** no replay in a held block, no switcher entry, no orchestrator dismiss call, no export to remind-me (owner gate), no retention cap. The scan is a heuristic.
