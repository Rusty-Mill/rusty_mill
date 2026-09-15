# 002 — Track P: errno comes back in the return register

Encountered wiring `rusty_libc` in as the Track P backend for
`sys::fdio::read`/`write` (platform-linux, D-12).

libc's contract for a failing call is a two-step dance: the function
returns `-1`, and the *code* lives in the thread-local `errno` — which is
why the libc-floor error path is `if n < 0 { read errno via
last_os_error() }`. The kernel's own contract is simpler and better: a
failing syscall returns `-errno` directly in the return register (the
range `-4095..=-1` is reserved for exactly this), and glibc's wrapper is
what splits that one value into the `-1` + thread-local pair. rusty_libc
undoes the split — `from_ret` decodes the register into
`Result<usize, Errno>` — so on the Track P path:

- The error code must flow **from the returned value**. Reading
  `last_os_error()` after a raw syscall is a bug: raw syscalls never touch
  glibc's `errno`, so it still holds whatever some *earlier* libc call
  left there. Hence the separate `trackp_err(op, Errno)` constructor
  beside `os_err(op, path)` rather than a shared error path.
- The unsafe block disappears from the call site. libc's `read` takes a
  raw pointer/length pair, so the caller asserts buffer validity; rusty_libc's
  wrapper takes `&mut [u8]` and derives the pair itself, keeping the
  unsafe (the asm) inside the dependency. The unsafe-scope gate doesn't
  move — it just gets emptier on our side of the boundary.
- Both paths converge on the same `kind_of(i32)` mapping and
  `OsCode::Errno`, so `PlatformError` is bit-identical either way — which
  is what lets the whole parity suite re-run under `--features track-p`
  as the equivalence test instead of needing a parallel suite.

Also of note: rusty_libc's MSRV is 1.88 (the x86_64 `SA_RESTORER`
trampoline needs stabilized naked functions), above this workspace's 1.75
floor. An off-by-default feature keeps the floor honest: the MSRV CI leg
never resolves the feature on, and only the ubuntu+stable leg runs the
Track P suite. Pinning by `rev` (not branch) keeps the dependency an
auditable snapshot — one source of truth upstream, no vendored fork to
drift.
