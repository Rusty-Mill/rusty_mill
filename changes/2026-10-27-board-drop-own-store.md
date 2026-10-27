---
category: Removed
changelog: Mill Board drops its own board file (`--store`) and backups (`--backup-dir`); its cards live in rusty_tick (ADR-0012 step 6).
---
## 2026-10-27 - Mill Board keeps its cards only in rusty_tick

- **Removed:** the `rusty_multimodal_db` board file (`--store`, `MILL_BOARD_STORE`), its one-owner lock, and the
  in-process backups (`--backup-dir`, `--backup-secs`, `--backup-keep`). The cards were migrated into rusty_tick
  and the board has run on `--tick-url`; back up rusty_tick's data directory instead. Without `--tick-url` the
  board lives in memory.
- **Fixed:** a card read from rusty_tick could pair a newer board document with an older revision when a write
  landed between the two reads, so an adopted execution link came back at a stale revision. A card read now
  checks the task's version again and starts over if it moved.
- **Changed:** the web UI no longer shows the `migrated:*` marker tag as a chip; it is still on the task.
- **Known limitations:** `list` still reads once, so a card in a listing can show that mix; a write made from it
  is refused as stale. Board events still live only in the process.
