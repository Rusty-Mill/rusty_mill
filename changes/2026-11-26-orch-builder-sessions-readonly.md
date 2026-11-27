---
category: Fixed
changelog: rusty_orch: `builder sessions` reads the store without opening it for writing, so inspecting it makes or repairs no file (#1318, D199).
---
## 2026-11-26 - rusty_orch: `builder sessions` is read-only on disk too

- **Fixed:** `rusty_orch builder sessions` (#1901) opened the orch store with the write-mode `Store::open`, so inspecting an empty state directory made a lock file and every table, and a partial store had its missing tables created. It now uses the new `Store::read_builder_sessions`, the portable read the lens reports use: the builder table and its records companion under the directory's existing lock, nothing written back. The writer path is unchanged.
- **Behaviour:** an empty `--state` (no store yet) prints nothing and stays empty. No lock, no builder table, no records companion or a corrupt companion exits 1.
- **Tests:** real-store fixtures check every entry and byte of the state dir is unchanged after a read, each partial case fails without repair, and two persisted terminal rows print exactly `tests/fixtures/builder-sessions.jsonl`.
