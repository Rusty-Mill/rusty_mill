# Release Notes

Per-PR record of what changed in `rusty_tokio` and why, newest first. This
repo has no version tags yet, so the unit of change is the merged PR.
`CHANGELOG.md` remains the semver-facing record of user-visible API changes;
this file carries the reasoning and the deliberate scope cuts behind them.

---

## Stop orphaning a socket when its Windows AFD-poll re-arm fails
**2026-09-03** · [Rusty-Mill/rusty_mill#153](https://github.com/Rusty-Mill/rusty_mill/issues/153)

- **Fixed:** unlike `epoll`/`kqueue`, the Windows reactor's `IOCTL_AFD_POLL`
  is one-shot and must be explicitly resubmitted after every completion.
  Both call sites in `Reactor::event_loop` discarded that resubmission's
  result (`let _ = self.submit_poll(&state)`); if it failed, no further
  completion would ever arrive for that socket, so any
  `readable()`/`writable()` wait registered on it afterward hung forever
  with nothing left to wake it.
- **How found:** `many_concurrent_tcp_connections` and three unrelated
  `rusty_tls` handshake tests were intermittently timing out at exactly
  nextest's ~600s slow-timeout on `test (windows-latest)`, then passing
  in milliseconds on the very next retry -- the signature of a lost
  wakeup, not a real failure. All four occurred *after* the earlier
  readiness-edge fix (#140) had already merged, so this is a distinct
  bug, not a recurrence of that one. Traced by reading
  `io/reactor/windows.rs`'s completion-handling loop rather than by
  reproducing locally -- this sandbox can cross-compile-check the crate
  for `x86_64-pc-windows-gnu` but cannot execute Windows tests, so real
  verification comes from `windows-latest` CI itself.
- **Fix:** both re-arm call sites now check `submit_poll`'s result and,
  on failure, mark both directions ready via a new `mark_orphaned`
  helper -- exactly the same "surface both directions so the caller's
  own next syscall discovers the truth" pattern `event_loop`'s sibling
  bad-completion-status branch already used, just extended to cover the
  re-arm-itself-failed case that branch didn't.
- **Verified:** `cargo check`/`clippy -D warnings` clean on
  `x86_64-pc-windows-gnu`; the full Linux `rusty_tokio` suite passes
  unaffected (the changed code is `#[cfg(windows)]`-only, so it isn't
  compiled or exercised on Linux at all).

---

## Stop losing readiness edges between a `WouldBlock` and its clear
**2026-09-02** · follow-up to [#138](https://github.com/Rusty-Mill/rusty_mill/pull/138)

- **Fixed:** every `WouldBlock` path (`poll_io`, `readiness::try_io`,
  `AsyncFdReadyGuard::clear_ready`) attempted the syscall and then cleared
  the cached readiness bit unconditionally. Under this crate's edge-triggered
  backends an edge delivered in between was wiped and never re-reported --
  the next wait on that direction hung. Measured as `tests/peek.rs::
  tcp_try_peek_fails_would_block_before_data_arrives_then_succeeds_after`
  stalling 1 run in 60 on main (identical with or without #138, so a
  separate bug from the connect one).
- **How:** each direction's readiness is now one word packing an edge
  counter above the ready bit (`ScheduledIo`), bumped atomically by every
  `mark_ready`. Callers take a `ReadyToken` snapshot before the syscall and
  clear through a single compare-and-swap against it; a bumped counter
  refuses the clear, the bit stays set, and the operation retries -- the
  same tick/generation idea tokio's `ScheduledIo` uses, in its smallest
  form. `AsyncFdReadyGuard` captures the token when handed out, so its
  `clear_ready` now matches tokio's documented "an event newer than the
  guard is not cleared" behaviour.
- **Why a CAS, not compare-then-store:** the reactor can bump the counter and
  set the bit between the two steps, and the store would still wipe it -- the
  exact window this change exists to close.
- **Verified:** unit tests on the token semantics (`reactor::tests`), the
  full Linux suite, and the peek binary run 100 times each before and after.

## Make `connect` wait for an in-flight non-blocking connect to resolve
**2026-09-02** · [Rusty-Mill/rusty_mill#137](https://github.com/Rusty-Mill/rusty_mill/issues/137)

- **Fixed:** `TcpStream::connect`/`TcpSocket::connect`/`UnixStream::connect`
  returned as soon as `SO_ERROR` read clean, which for a connect still in
  flight is immediately -- the reactor registers every fd optimistically
  writable, so the "wait for writability, then check `SO_ERROR`" sequence
  never actually waited. A refused loopback connect on Windows (always
  asynchronous there) therefore never reported anything; rusty-meshed's
  "unreachable Kafka" tests sat until nextest's 600s kill.
- **How:** the platform `connect` helpers now say whether the connect was
  established inside the call or is still in progress
  (`socket::ConnectOutcome`), and an in-progress one is registered with a
  cleared writable bit (`reactor::InitialReadiness::WritePending`, a new
  `register_with` on every backend). Chosen over clearing the bit after
  registration because `epoll` (`EPOLLET`) and `kqueue` (`EV_CLEAR`) are
  edge-triggered here: an edge delivered between registration and a later
  clear would be lost for good. All four backends report an fd's current
  state at registration time, so a connect that completes early is still
  observed.
- **Also fixed, found on the way:** `epoll`/`kqueue` armed the kernel
  registration before inserting the fd into the reactor's registry, so an
  edge dequeued in that window was dropped -- harmless while every wait
  started optimistic, a permanent hang once a connect genuinely waits for
  its first edge (one stall in ~20 parallel runs of the new tests). Both now
  insert first, and roll the entry back if the kernel call fails; `io_uring`
  and Windows already did it in that order.
- **Why it was missed:** a Linux loopback `connect(2)` reports `EINPROGRESS`
  but has already processed the handshake (or the RST) inside the call, so
  the premature `SO_ERROR` check saw a settled answer; for a remote peer the
  first `send` in `SYN_SENT` returns `EAGAIN`, so the reactor wait silently
  moved to the first write. Every existing test passed either way.
  The only refused-connect test upstream (`rusty_request`) rebinds the port
  20ms later, inside Windows' SYN-retry window, so it passed there too.
- **Verified:** Linux test suite plus `cargo check --target
  x86_64-pc-windows-gnu`; the Windows arm's runtime behaviour is validated by
  the new `tests/tcp_connect_refused.rs` on the Windows CI leg, since this
  environment has no Windows hardware.

## Normalize line endings on the generated governance files
**2026-08-15** · PR [#271](https://github.com/baileyrd/rusty_tokio/pull/271)

- **Fixed:** `SECURITY.md`, `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, and
  `docs/adr/0001-template.md` landed with CRLF line endings. They were copied
  verbatim from a tooling skill whose entire template payload is CRLF, and it
  went unnoticed because the two files rewritten by hand
  (`ARCHITECTURE.md`, `RELEASE_NOTES.md`) came out LF and looked
  representative. Now all LF.
- **Added:** `.gitattributes` with `* text=auto eol=lf`. The generator still
  emits CRLF, so without this the next regeneration reintroduces it — fixing it
  at the repo boundary is what makes the fix durable rather than a one-off
  cleanup.

## Correct the `syn` row in `dependency-audit.md`
**2026-08-15** · [#268](https://github.com/baileyrd/rusty_tokio/issues/268) · [#270](https://github.com/baileyrd/rusty_tokio/pull/270) (closed unmerged)

- **Fixed:** the audit claimed `syn`/`quote`/`proc-macro2` were "the only row
  with a real path to elimination." They aren't. Removing them from
  `rusty_tokio-macros` leaves them in the build anyway, via `platform` →
  `thiserror` → `thiserror-impl` — 38 lockfile packages either way.
- **Why it was missed:** the audit's scope was direct, non-dev, non-build
  dependencies. A row can look eliminable at that level while remaining in the
  graph transitively, so hand-roll candidates now need a `cargo tree -i <crate>`
  check before being called eliminable.
- **Outcome:** a working hand-rolled replacement was built and verified (span
  fidelity preserved, all five diagnostics located, 11 new tests), then declined
  — ~250 lines of token parsing to maintain for no change in what compiles. The
  branch is preserved on closed PR #270. `syn`/`quote`/`proc-macro2` stay.
- **Follow-up identified:** `thiserror` in `rustils`' `platform` crate is the
  single dependency keeping `syn`, `quote`, `proc-macro2`, and `unicode-ident`
  in every consumer of the platform layer. That's the real lever, and it's in a
  different repo.
- Documentation only — no code change.

## Gate `bytes` behind a Cargo feature
**2026-08-15** · [#267](https://github.com/baileyrd/rusty_tokio/issues/267) · PR pending

- **Changed:** `bytes` is now `optional = true` behind a `bytes` feature, off by
  default. A default build of `rusty_tokio` now pulls **no optional external
  crates at all** — only `crossbeam-deque` and the deliberate
  `libc`/`windows-sys` floor.
- **Why this is a gating and not a removal:** `bytes` is used at 13 sites and
  every one is a generic bound (`B: bytes::BufMut`), never a concrete type. The
  dependency *is* the interop contract — the point is accepting a caller's own
  `bytes::BytesMut`. `rusty_wire` was evaluated as an internal replacement and
  rejected: it's a concrete byte cursor, not the `Buf`/`BufMut` trait ecosystem,
  so swapping it in would break the feature rather than internalize it. See
  `dependency-audit.md`.
- **Breaking, but asymmetrically so** (`ATLAS-IFACE-0001`): the four gated
  ext-trait methods (`read_buf`, `write_buf`, `write_all_buf`, plus
  `UdpSocket::{recv_buf, recv_buf_from}` and `TcpStream::try_read_buf`) are
  *provided* methods, so gating them off is **not** a breaking change for
  anything implementing `AsyncRead`/`AsyncWrite` — only for callers of those
  methods, who now need `features = ["bytes"]`.
- **Changed:** `MAX_UDP_DATAGRAM_SIZE` stays ungated — the cap is a property of
  UDP and is useful to callers sizing their own buffers regardless. Its doc links
  to `bytes::BufMut` became plain code spans so they don't dangle when the
  feature is off.
- **Changed:** CI now runs the default build and the feature-enabled build as
  separate steps. The default step is what keeps the no-optional-crates claim
  honest and must not be folded into the other.
- Verified: 59 test targets pass by default (`tests/buf.rs` correctly excluded,
  `bytes` absent from `cargo tree -e normal`); 60 pass with `--features bytes`,
  including all 9 `buf` tests.

## Repo governance file set + dependency sovereignty audit
**2026-08-15** · PR pending

- **Added:** the standard governance file set — `CONTRIBUTING.md`,
  `SECURITY.md`, `CODE_OF_CONDUCT.md`, `ARCHITECTURE.md`, this file, an ADR
  seed under `docs/adr/`, and `.github/` PR + issue templates. The repo scored
  2/10 against the standard set before this (only `README.md` and
  `CHANGELOG.md`).
- **Added:** `.github/workflows/ci-rust.yml`. Adapted rather than taken stock:
  the standard workflow's `cargo test --workspace` silently skips every test
  target carrying `required-features`, which here means the `futures-io-compat`
  and `tracing` suites never run. Those now run explicitly. The io_uring and
  `thread-per-core` features are compile-checked but **not** run — hosted
  runners vary in whether their seccomp profile permits `io_uring_setup`, and a
  flaky gate is worse than an honest compile-only one. That is a real coverage
  gap, not a solved problem: nothing in CI exercises the io_uring paths.
- **Added:** `dependency-audit.md` — classification of all six external
  dependencies against internal (`rustils`, `rustils_async`, `rusty_sync`,
  `rusty_wire`) coverage. Conclusion: one genuine drop candidate
  (`syn`/`quote`/`proc-macro2`), three interop contracts where the external
  crate *is* the deliverable (`bytes`, `futures-io`, `tracing`), and two
  documented decisions left standing (`crossbeam-deque`, `io-uring`).
  `libc`/`windows-sys` excluded as the deliberate rustils RFC v2 floor.
- **Fixed:** `udp_socket_bind_device_set_and_read_back_then_cleared`
  (`tests/net.rs`) and `bind_device_set_and_read_back_then_cleared`
  (`tests/tcp_socket.rs`) both failed with `EPERM` the moment CI first ran
  them. Pre-existing — they had simply never run in CI before. The kernel
  gates `SO_BINDTODEVICE` on `CAP_NET_RAW` only when the bound ifindex
  actually changes, so binding to `lo` succeeds unprivileged and only the
  final clear does not. Each test now tolerates that one error on that one
  call and skips the clear half; any other error still panics. **Known
  limitation:** nothing exercises the unbind path on an unprivileged runner
  — a gap CI cannot close without elevated capabilities.
- **Known limitation:** `ARCHITECTURE.md` notes that several load-bearing
  decisions (#8's `crossbeam-deque` adoption, #9's io_uring scope limit, the
  Windows socket-layer split) still live in issue threads and manifest
  comments rather than ADRs. The ADR directory is seeded with the template
  only; migrating those is outstanding.
- No behavior change — documentation, CI, and templates only.
