# Release Notes

Per-PR record of what changed in `rusty_tokio` and why, newest first. This
repo has no version tags yet, so the unit of change is the merged PR.
`CHANGELOG.md` remains the semver-facing record of user-visible API changes;
this file carries the reasoning and the deliberate scope cuts behind them.

---

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
