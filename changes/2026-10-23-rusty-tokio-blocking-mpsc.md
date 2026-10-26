---
category: Added
changelog: rusty_tokio mpsc gains `blocking_send`/`blocking_recv` for plain OS threads, and `AsyncFd` implements `AsRawFd`.
---
## 2026-10-23 - rusty_tokio: blocking mpsc, `AsyncFd: AsRawFd`

- **Added:** `mpsc::Sender::blocking_send`, `mpsc::Receiver::blocking_recv` and `mpsc::UnboundedReceiver::blocking_recv`, tokio-shaped, so a plain OS thread (a blocking `ReadFile`/`WriteFile` loop, say) can bridge into async tasks. They park the calling thread on the runtime's existing `block_on` parker and need no runtime. Needed to move rusty_term off tokio.
- **Added:** `impl AsRawFd for AsyncFd<T>`, as tokio has, so a `try_io` closure can syscall on its argument.
- **Known limitations:** unlike tokio, calling them from inside an async task does not panic; it parks that worker thread. The docs say not to.
