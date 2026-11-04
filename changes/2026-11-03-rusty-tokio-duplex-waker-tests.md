---
category: Added
changelog: rusty_tokio DuplexStream gains regression tests for write-waker replacement and drop waking a blocked writer.
---
## 2026-11-03 - rusty_tokio DuplexStream waker regression tests

- **Added:** two deterministic tests in `rusty_tokio/tests/duplex.rs`, suggested on the #1177
  review (card #440): a writer re-polled with a new waker is woken through that one and not the
  replaced one, and dropping the reading half wakes a blocked writer, whose next write fails
  with `BrokenPipe`. Tests only; no behaviour change.
