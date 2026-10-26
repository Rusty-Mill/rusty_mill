---
category: Changed
changelog: The rusty_term window parses PTY output in batches of up to 64 KiB and wakes the UI once per handled batch, instead of once per 4 KiB read.
---
## 2026-10-26 - rusty_term: batched PTY reads and coalesced wakeups

- **Changed:** the window's reader thread reads into one reused 64 KiB buffer. After a blocking read it keeps reading while the PTY has more output ready (`poll` with a zero timeout on Unix) and parses the whole batch under one grid lock. A Linux PTY master returns at most about 4 KiB a read, so a flood used to cost one lock and one parse per 4 KiB. On an 8 MB `cat`, parse calls drop from 4,200–8,200 to 150–230. Headless throughput is unchanged within noise (40–78 MB/s either way).
- **Changed:** a pane's reader sends a `Redraw` wakeup only when the window has handled the last one, so a flood costs the UI thread about one event per loop iteration rather than one per read. Each event used to lock the grid several times on the UI thread.
- **Added:** `BackendHandle::read_into` (reads into a caller's buffer; `read` now wraps it), `BackendHandle::output_ready` (false by default, so Windows batches by the size of one pipe read) and `backend::read_batch`.
- **Tested:** `read_batch` against a scripted handle (joining ready reads, a full buffer, an error after bytes and a first-read error), and every byte of 1 MB through a real PTY. An ignored release measurement compares batched and per-read parsing through a PTY.
- The terminal-mode (TUI) reactor path is unchanged; its frame notifications already coalesce.
