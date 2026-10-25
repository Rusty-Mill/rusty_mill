---
category: Changed
changelog: The rusty_term handoff docs record that rusty_mux_core (#953) and rusty_mux_proto (#982) exist.
---
## 2026-10-24 - rusty_term handoff docs: first mux crates

- **Changed:** `docs/projects/mux/RUSTY-TERM-NOTES.md` no longer says no `rusty_mux_*` crate exists. It names `rusty_mux_core` (#953) and `rusty_mux_proto` (#982), notes that the server is still the open #994, and records that the protocol's `Snapshot` message is full-state only. `RUSTY-TERM-HANDOFF.md`'s incremental-snapshot step says the same.
- **Known limitations:** documentation only. #994 (server, plus a new terminal checkpoint format) is not merged and is not described.
