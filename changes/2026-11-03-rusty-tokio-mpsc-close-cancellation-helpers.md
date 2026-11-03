---
category: Added
changelog: rusty_tokio adds mpsc Receiver::close and CancellationToken::run_until_cancelled(_owned) and drop_guard_ref.
---
## 2026-11-03 - rusty_tokio mpsc close and CancellationToken helpers

- **Added:** `mpsc::Receiver::close` and `mpsc::UnboundedReceiver::close`, as tokio's: later sends
  fail, waiting senders wake to see that, and queued values (and values sent through a permit
  taken before) are still received; `recv` returns `None` once drained (card #406).
- **Added:** `CancellationToken::run_until_cancelled`, `run_until_cancelled_owned` and
  `drop_guard_ref` (with `sync::DropGuardRef`), as tokio-util's. Cancellation wins when both
  are ready.
- **Known limitations:** `#[rusty_tokio::test]` still defaults to the multi-thread flavor where
  tokio defaults to current-thread; changing it alters existing tests' behaviour and is left
  for an owner decision.
