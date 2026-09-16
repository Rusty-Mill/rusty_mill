# 001 — fdopendir takes ownership of its fd

Encountered while implementing `sys::fdio::read_dir` (platform-linux).

`fdopendir(3)` *consumes* the descriptor: after a successful call, the fd
belongs to the `DIR*` stream and is closed by `closedir` — closing it
yourself is a double-close. In Rust terms, `OwnedFd` must relinquish
ownership (`into_raw_fd`) at exactly that point, and the failure path must
NOT relinquish (the fd is still ours to drop). The first draft used
`std::mem::forget` on the raw fd — harmless for an `i32` but semantically
wrong and flagged by `forgetting_copy_types`; `into_raw_fd`'s return value,
deliberately discarded, states the transfer precisely.

Also of note: a `Dir` capability cannot hand its *own* fd to `fdopendir`
and stay usable — enumeration therefore opens a fresh fd on `"."` relative
to itself. Directory streams and directory capabilities are different
lifetimes wearing the same fd type.
