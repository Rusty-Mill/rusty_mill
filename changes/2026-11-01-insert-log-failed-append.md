---
category: Fixed
changelog: rusty_multimodal_db_engine's insert log no longer loses an acknowledged write after a failed append, and refuses further writes after a failed sync.
---
## 2026-11-01 - insert log: a failed append no longer buries the next one

- **Fixed:** a write that failed partway left partial bytes in `<path>.inserts`.
  The next append landed after them, and reopen dropped it as part of the
  torn tail, so an acknowledged insert was lost. A failed write now truncates
  the log back to its last complete entry (a log the append created is
  removed), and the next append starts clean. Path-based appends (edge logs)
  get the same rollback.
- **Fixed:** after a failed `sync_data`, a retry on a fresh handle could report
  success for data the kernel had already dropped. A failed sync, or a
  failed rollback, now poisons the `Appender`: every later append or sync
  returns an error until the store is reopened. `Appender::is_poisoned`
  reports it.
- **Known limitations:** edge-log framing is unchanged; the paged-storage
  design covers it.
