---
category: Added
changelog: rusty_orch: read-only `rusty_orch builder sessions`, printing the builder sessions the orch store holds (#1318, D199).
---
## 2026-11-26 - rusty_orch: `builder sessions` (#1318)

- **Added:** `rusty_orch builder sessions <config> --dir --task --state [--claim]` prints the board's persisted builder sessions, one JSON line each in `builder run`'s row shape without notes, ordered by claim and n. It is read-only: it waits for the store lock like `builder run` and changes no session. It exits 1 when `--state` is not a directory (it never creates one) or the store can't be read, and 2 on a bad command line.
- **Why:** `builder run` prints its row even when its final save failed, so the D167 toy-run verdict (#1894) checks what the store holds instead. Approved by Nano for this one command (D199).
- **Changed:** `builder run`'s flag parsing is shared with `builder sessions`; its flags and errors are unchanged.
