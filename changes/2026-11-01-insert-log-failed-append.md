---
category: Fixed
changelog: rusty_multimodal_db_engine's insert and edge logs no longer lose an acknowledged write after a failed append, and refuse further writes after a failed rollback or sync.
---
## 2026-11-01 - insert log: a failed append no longer buries the next one

- **Fixed:** a write that failed partway left partial bytes in `<path>.inserts`.
  The next append landed after them, and reopen dropped it as part of the
  torn tail, so an acknowledged insert was lost. A failed write now truncates
  the log back to its last complete entry and syncs the new length (a log
  the append created is removed), and the next append starts clean. The
  truncate uses its own write handle, since a Windows append handle cannot
  truncate.
- **Fixed:** after a failed `sync_data`, a retry on a fresh handle could report
  success for data the kernel had already dropped. A failed sync, or a
  failed rollback, now poisons the `Appender`: every later append or sync
  returns an error until the store is reopened. `Appender::is_poisoned`
  reports it, and `Appender::close` syncs pending deferred entries through
  their own handle first (it now returns `Result`).
- **Fixed:** `Symmetric` and `MultiSymmetric` edge logs write through an
  `Appender` per log, so the same fence holds for links and edge tombstones.
- **Changed:** `Appender::close` returns `Result<(), DurabilityError>`, and
  `Appender::append_item` is new.
