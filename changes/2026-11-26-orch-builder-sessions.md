---
category: Added
changelog: rusty_orch: read-only `rusty_orch builder sessions`, printing the builder sessions the orch store holds (#1318, D199).
---
## 2026-11-26 - rusty_orch: `builder sessions` (#1318)

- **Added:** `rusty_orch builder sessions <config> --dir --task --state [--claim]` prints the board's persisted builder sessions, one JSON line each in `builder run`'s row shape without notes, ordered by claim and n. It is read-only: it reads the builder table and its records companion under the store's existing lock (the portable read the lens reports use, new `Store::read_builder_sessions`), never `Store::open`, so it makes no lock file or table and writes nothing back. An empty `--state` directory (no store yet) prints nothing. It exits 1 when `--state` is not a directory (it never creates one), or holds a partial store (no lock, no builder table or no records companion), a corrupt one, or one that stays locked; 2 on a bad command line.
- **Why:** `builder run` prints its row even when its final save failed, so the D167 toy-run verdict (#1894) checks what the store holds instead. Approved by Nano for this one command (D199).
- **Changed:** `builder run`'s flag parsing is shared with `builder sessions`; its flags and errors are unchanged.
