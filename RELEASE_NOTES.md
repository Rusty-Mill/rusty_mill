# Release Notes

One entry per merged PR against `main`, reverse chronological. No version tags
exist yet (still pre-1.0/unreleased on crates.io), so this tracks by PR rather
than by tag — see `CHANGELOG.md` for the `[Unreleased]` rollup once a tag ships.

---

## net: add peer_addr_unix, closing #271
**2026-08-08** · branch `claude/rustils-windows-deps-mtimc7` (no PR number yet)

- **Added:** `net::peer_addr_unix` — `getpeername` for `AF_UNIX` sockets.
  `local_addr_unix` (added alongside the rest of the `AF_UNIX` slice)
  had no peer counterpart, unlike the IP side's `local_addr`/`peer_addr`
  pair. Filed as #271 by rustils, whose `sys::net` Track W migration
  (baileyrd/rustils#106) needed exactly this and had no upstream binding
  to route through.
- **Behavior worth knowing:** an `accept`ed `AF_UNIX` peer is commonly
  reported *unnamed* — an empty path, not an error. Windows does not
  autobind a connecting client to a path the way it assigns an ephemeral
  port to a TCP client, so unless the client explicitly bound itself
  first, there is nothing to report. Documented on the function; a new
  test (`peer_addr_unix_reports_the_client_as_unnamed`) pins the
  behavior rather than leaving it as an assumption.
- **Verification:** two new live round-trip tests
  (`peer_addr_unix_reports_the_client_as_unnamed`,
  `peer_addr_unix_matches_local_addr_unix_from_the_other_side`) drive a
  real bind/listen/connect/accept sequence over a `std::env::temp_dir()`
  path — the first live-socket coverage this module's `AF_UNIX` slice
  has had; every existing test there only encoded/decoded
  `UnixSocketAddr` bytes. Both are Windows-only (`cfg(all(test,
  windows))`), so they run on the `windows-latest` CI job, not the
  cross-compile pre-check.

---

## net/security bindings for rustils' track-w backend
**2026-08-07** · branch `claude/rustils-windows-deps-mtimc7` (no PR number yet)

- **Added:** `crypto` (`BCryptGenRandom`), `credential` (Credential
  Manager, `CRED_TYPE_GENERIC`), and `certstore` (system certificate
  stores, read-only) as three new modules, plus `net::set_nonblocking`
  (`ioctlsocket(FIONBIO)`) and AF_UNIX support in `net`
  (`AddressFamily::Unix`, `Protocol::Unspecified`, `UnixSocketAddr`,
  `bind_unix`/`connect_unix`/`accept_unix`/`local_addr_unix`).
- **Why:** rustils adopted this crate as its Windows backend behind
  `platform-windows`'s `track-w` feature and migrated everything it
  could; what remained on `windows-sys` was blocked on bindings that
  simply did not exist here. This closes that set. `security` was
  deliberately not extended — it answers "who may touch this object"
  (ACLs, SIDs), which is a different job from "give me unpredictable
  bytes", "hold this secret" and "whom does this machine trust". Three
  modules, three clear scopes, rather than one with four error
  conventions.
- **Note:** `crypto::random_bytes` returns `Result<usize, NtStatus>`, the
  only wrapper here that does not return `Win32Error`. `BCryptGenRandom`
  reports an `NTSTATUS`, and unlike `conpty`'s `HRESULT`s (which embed a
  recoverable Win32 code) there is no faithful conversion without an
  `ntdll` export this crate does not bind. Returning the raw status typed
  is honest; widening it into `Win32Error` would have put an NTSTATUS bit
  pattern into a field documented to hold a Win32 error code.
- **Verification:** `CREDENTIALW`, `CERT_CONTEXT` and `sockaddr_un`
  layouts transcribed from the Windows metadata and pinned with
  `size_of`/`align_of`/`offset_of` compile-time asserts. New unit tests
  cover the CSPRNG (fills, differs, empty is a no-op), credential
  round-trip/replace/clean-miss/binary-safety, ROOT-store enumeration
  (asserting each entry really is a DER SEQUENCE, not merely non-empty),
  and `UnixSocketAddr` validation. All Windows-only, so the
  windows-latest job is where they run.

## manifest: edition 2021 + declared 1.88 MSRV, so rustils can depend on this crate
**2026-08-07** · branch `claude/rustils-windows-deps-mtimc7` (no PR number yet)

- **Changed:** `edition = "2024"` → `"2021"`, and `rust-version = "1.88"`
  declared for the first time. This is a packaging change, not a language
  one — no source file changed. Cargo parses the manifest of every
  resolved dependency, *including an optional one whose feature is off*,
  so an edition-2024 manifest here fails `cargo check` outright for any
  consumer on a pre-1.85 toolchain. rustils keeps a 1.75 MSRV leg in its
  CI matrix and now wires this crate in as an optional, feature-gated
  Windows backend (`platform-windows`'s `track-w`, off by default), which
  is only viable if the dependency's mere presence costs that floor
  nothing. `rusty_libc` — adopted into rustils the same way, behind
  `track-p` — already has exactly this shape.
- **Changed:** `[lints.rust] unsafe_op_in_unsafe_fn = "deny"` added, and
  `rustfmt.toml` added pinning `style_edition = "2024"`. Both exist to
  make the edition drop a no-op in every respect except manifest
  parsing: the first keeps edition 2024's "an `unsafe fn` body is not an
  implicit `unsafe` block" rule (already how this crate is written
  throughout), the second stops rustfmt — whose style edition otherwise
  follows the crate edition — from reflowing the entire codebase.
- **Added:** an `msrv` CI job (Rust 1.88, cross-compiled to
  `x86_64-pc-windows-gnu`). A `rust-version` nobody compiles against
  drifts upward silently on the first use of a newer library API, and
  this one is now load-bearing for a real consumer. The 1.88 figure was
  measured, not guessed: `impl Default for *mut T` stabilized there, and
  `#[derive(Default)]` on the OVERLAPPED/handle-bearing structs in
  `watch`/`net`/`conpty` needs it — 1.87 fails, 1.88 is clean.

---

## PR #266 — conpty: expose CreatePseudoConsole's PSEUDOCONSOLE_INHERIT_CURSOR flag
**2026-08-05** · [#266](https://github.com/baileyrd/rusty_win32/pull/266)

- **Added:** `conpty::create_with_flags` plus `conpty::PSEUDOCONSOLE_INHERIT_CURSOR`,
  closing issue #263 — a gap found by a fresh, narrow parity-loop pass
  (`gap-analysis.md`, PR #265) scoped to just the `conpty` module against the
  real Win32 ConPTY surface, read directly from mingw-w64's headers rather
  than assumed from memory. `conpty::create` had always hardcoded
  `CreatePseudoConsole`'s `dwFlags` to `0`, so `PSEUDOCONSOLE_INHERIT_CURSOR`
  — the only flag Windows documents for this function — could never be
  requested. It makes the new pseudoconsole inherit the parent terminal's
  current cursor position instead of starting at `(0,0)`, useful when
  creating a pseudoconsole mid-session (e.g. after a resize/re-attach) so a
  hosted application's rendering continues from where the parent left off.
  `create_with_flags` is a wholly new function alongside `create` (verified
  non-breaking: no change to `create`'s existing public signature); `create`
  now forwards to `create_with_flags(..., 0)`, the same hardcoded value it
  always passed.

---

## PR #259 — process: add spawn_suspended_with_pseudoconsole
**2026-07-24** · [#259](https://github.com/baileyrd/rusty_win32/pull/259)

- **Fixed:** CI caught a real bug in this same PR's first push: the
  `AttributeList::update` call binding `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE`
  passed `&hpc` (the address of the local `Hpcon` variable) instead of
  `hpc` itself (the handle's own pointer-sized value). Both compile and
  type-check identically (`*const c_void`-shaped either way), so nothing
  caught it locally — the spawned `cmd.exe /c exit 7` child ran but
  failed its own console-subsystem startup with `STATUS_DLL_INIT_FAILED`
  (`0xC0000142`), reported as its real exit code by the test's own
  `wait` call. Windows' documented usage for this one attribute (and its
  official ConPTY sample) passes the `HPCON` value directly as
  `lpValue`, unlike most other process-thread attributes which take a
  pointer to a variable holding the value — fixed by passing
  `hpc.cast_const()` instead of `(&hpc as *const Hpcon).cast()`.
- **Added:** `process::spawn_suspended_with_pseudoconsole` plus
  `StartupInfoExW`/`EXTENDED_STARTUPINFO_PRESENT`, closing issue #189 —
  a wholly new function alongside `process::spawn_suspended` (verified
  non-breaking: no change to that function's existing public signature)
  that starts a command suspended, hosted by an already-created
  pseudoconsole (`conpty::create`, PR #257), for a fully-interactive
  child rather than one with redirected stdio pipes. Builds a one-
  attribute `conpty::AttributeList` (PR #258), binds the caller's
  `Hpcon` in via `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE`, then calls
  `CreateProcessW` with a `STARTUPINFOEXW` — `EXTENDED_STARTUPINFO_PRESENT`
  set, `cb` sized for the *extended* struct (`CreateProcessW`'s own
  documented requirement once that flag is set) — instead of a bare
  `StartupInfoW`, and drops `spawn_suspended`'s `inherit_handles`
  parameter (irrelevant once a pseudoconsole supplies the child's
  console I/O; hardcoded `FALSE`). `STARTUPINFOEXW` (112 bytes, 8-byte
  aligned, `lpAttributeList` at offset 104 — exactly `StartupInfoW`'s
  104 bytes plus one trailing pointer) and `EXTENDED_STARTUPINFO_PRESENT`
  (`0x0008_0000`) verified via a compiled mingw-w64 probe that also
  exercises the real `CreateProcessW` call shape. Added a `pub(crate)`
  `AttributeList::as_mut_ptr` accessor (`conpty.rs`) since this is that
  type's first real, non-test caller. The attribute list is torn down
  via `Drop` before this function returns either way; only the caller's
  `Hpcon` outlives the call, for `conpty::resize`/`conpty::close`
  afterward. Tested end-to-end: a real pipe pair, `conpty::create`, this
  function spawning `cmd.exe /c exit 7`, confirming `CREATE_SUSPENDED`
  actually held the thread (a zero-timeout `wait` reports no exit before
  `resume`), then `resume`/`wait` reporting the real exit code, matching
  `spawn_suspended`'s own existing round-trip test shape. This closes
  out the `conpty` module's round-2 scope (issues #187-190), and with
  it this entire round-2 parity-loop batch.

## PR #258 — conpty: add process-attribute-list plumbing
**2026-07-24** · [#258](https://github.com/baileyrd/rusty_win32/pull/258)

- **Added:** `conpty::AttributeList` (`InitializeProcThreadAttributeList`/
  `UpdateProcThreadAttribute`/`DeleteProcThreadAttributeList`), closing
  issue #188 — the generic (pre-ConPTY, Vista-era) extended-process-
  attribute mechanism, the only way to hand an `Hpcon` to
  `CreateProcessW` (via `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE`, from PR
  #257). `PROC_THREAD_ATTRIBUTE_LIST`'s true byte size is knowable only
  at runtime: `AttributeList::init` calls
  `InitializeProcThreadAttributeList` once with a null buffer (its
  documented size-query mode) to discover the required size, then again
  on a freshly allocated `Vec<u8>` to do the real initialization — a
  query-then-allocate opaque-byte-buffer pattern, new territory for this
  crate, distinct from its existing "retry a UTF-16 string buffer at the
  size the API reports" idiom (`console::title`,
  `service::display_name`/`key_name`). `SIZE_T`/`DWORD_PTR` widths (both
  8 bytes) and the full three-function signature set verified via a
  compiled mingw-w64 probe. `AttributeList::delete`
  (`DeleteProcThreadAttributeList`) is idempotent — a second explicit
  call, or the `Drop` impl running after one, is a no-op rather than a
  double-free. Tested `init`/`delete` alone, plus a full integration
  test binding a real `Hpcon` (from `conpty::create`) into a freshly
  initialized attribute list via `update`, confirming
  `UpdateProcThreadAttribute` itself succeeds — the strongest check
  available without `CreateProcessW` (issue #189, next).

## PR #257 — conpty: add pseudoconsole lifecycle + Hpcon
**2026-07-24** · [#257](https://github.com/baileyrd/rusty_win32/pull/257)

- **Added:** `conpty` module (new subsystem): `conpty::create`/
  `conpty::resize`/`conpty::close` (`CreatePseudoConsole`/
  `ResizePseudoConsole`/`ClosePseudoConsole`), closing issue #187, plus
  `Hpcon`/`PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE`, closing issue #190 —
  folded into this same PR since `create`'s own signature
  (`Result<Hpcon, Win32Error>`) already needs the `Hpcon` type issue
  #190 owns, the same real, signature-level dependency that combined
  `net::bind`/`SocketAddr` into one PR earlier in round 2 (issue #175 +
  #185, PR #246). `HPCON` (8-byte pointer-shaped), `COORD` (reused from
  `console::Coord`), and `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE`
  (`0x0002_0016`, from Windows' own `ProcThreadAttributeValue` macro)
  all verified via a compiled mingw-w64 probe, built with
  `-D_WIN32_WINNT=0x0A00 -DWINVER=0x0A00 -DNTDDI_VERSION=0x0A000006` to
  see past `CreatePseudoConsole`/`ResizePseudoConsole`/
  `ClosePseudoConsole`'s version guard (a C-header-only concern — the
  real symbols link fine regardless, confirmed present in
  `libkernel32.a` via `nm`). `CreatePseudoConsole`/`ResizePseudoConsole`
  are this crate's only `HRESULT`-returning functions (everything else
  reports failure via ordinary `GetLastError()`); both unwrap that
  `HRESULT` back to the plain Win32 error code it was built from via
  `HRESULT_FROM_WIN32`'s well-defined, reversible bit layout, rather
  than exposing a raw `HRESULT` this crate's `Win32Error`/
  `FormatMessageW` machinery wouldn't recognize. Tested the full
  create/resize/close lifecycle against a real pipe pair, including the
  documented "caller closes its own copies of the input/output handles
  right after `CreatePseudoConsole` succeeds" pattern (ConPTY duplicates
  them internally). `InitializeProcThreadAttributeList`/
  `UpdateProcThreadAttribute`/`DeleteProcThreadAttributeList` (issue
  #188) and `process::spawn_suspended_with_pseudoconsole` (issue #189)
  are next.

## PR #256 — net: add byte-order conversions
**2026-07-24** · [#256](https://github.com/baileyrd/rusty_win32/pull/256)

- **Added:** `net::htons`/`net::htonl`/`net::ntohs`/`net::ntohl`, closing
  issue #186 — thin wrappers over the real Winsock functions of the
  same name (verified via a compiled mingw-w64 probe: `u_short`/
  `u_long` are 16/32-bit, no socket parameter). Same lowercase-symbol
  collision as the rest of this module's Winsock wrappers — bound via
  `#[link_name = "htons"]`/`"htonl"`/`"ntohs"`/`"ntohl"` on distinctly-
  named `raw_htons`/`raw_htonl`/`raw_ntohs`/`raw_ntohl` externs. Unlike
  every other function in this module, these are pure byte-swaps with
  no socket/Winsock-lifecycle dependency — no `startup()` call needed
  first, and the crate-internal `to_sockaddr`/`from_sockaddr` already
  apply the same conversion internally via `u16::to_be`/`from_be`; this
  is the public, standalone equivalent for callers working with their
  own raw wire fields. This closes out the `net` module's round-2 scope
  (issues #173-186, `#185` folded early into PR #246).

## PR #255 — net: add resolve
**2026-07-24** · [#255](https://github.com/baileyrd/rusty_win32/pull/255)

- **Added:** `net::resolve` (`getaddrinfo`/`freeaddrinfo`) plus
  `AddrInfoHints`/`ResolvedAddr`, closing issue #184 — hostname/
  service-name to address resolution. The real `addrinfo` layout (48
  bytes, 8-byte aligned; every field's offset) verified via a compiled
  mingw-w64 probe. `resolve` walks Windows' own returned linked list,
  copies each entry's family/socktype/protocol/address out into an
  owned `Vec<ResolvedAddr>` (reusing the existing `from_sockaddr`
  decoding), then calls `freeaddrinfo` before returning — nothing
  borrowed from Windows-owned memory escapes the call. An entry whose
  family/socktype/protocol falls outside this module's supported set is
  silently skipped rather than failing the whole resolution, the same
  "explicitly out of scope" policy already applied at the `socket`/
  `bind` boundary. Unlike the rest of this module's `WSAGetLastError`-
  reporting functions, failure comes back via `getaddrinfo`'s own return
  value directly (`EAI_*` codes are aliases of the ordinary `WSA*` error
  codes on Windows) — the same convention `startup`/`WSAStartup`
  already uses. First user of `alloc::vec::Vec` in this module (every
  earlier `net` function used fixed-size stack buffers), needed since
  the result count is unbounded. Tests resolve numeric IPv4/IPv6
  hosts and a numeric port (no real name resolution, so no network
  dependency), confirm `AF_UNSPEC` (`hints.family: None`) still resolves
  a numeric IPv4 host, and confirm a made-up, non-numeric service name
  fails without needing network access.

## PR #254 — net: add local_addr/peer_addr
**2026-07-24** · [#254](https://github.com/baileyrd/rusty_win32/pull/254)

- **Added:** `net::local_addr`/`net::peer_addr` (`getsockname`/
  `getpeername`), closing issue #183 — read back a socket's own bound
  address (e.g. after binding to an OS-assigned ephemeral port `0`) and
  a connected socket's peer address. Both reuse the existing
  `from_sockaddr` decoding from PR #246, with the same fixed 28-byte
  `sockaddr`-shaped out-buffer `accept`/`recvfrom` already use. Same
  no-collision situation as `set_sockopt`/`get_sockopt` (PR #253): the
  real `getsockname`/`getpeername` symbols don't clash with this
  module's `local_addr`/`peer_addr` wrapper names, so no `#[link_name]`
  was needed. Tested `local_addr` after binding to port `0` (asserting
  the reported port is non-zero — the whole point of the call), and
  `peer_addr` plus `local_addr` on a real local TCP connection (the
  client's `peer_addr` matches the server's bound address; the accepted
  socket's `local_addr` also matches, since it inherits the listening
  socket's local address).

## PR #253 — net: add sockopt
**2026-07-24** · [#253](https://github.com/baileyrd/rusty_win32/pull/253)

- **Added:** `net::set_sockopt`/`net::get_sockopt` (`setsockopt`/
  `getsockopt`) plus `SockOpt`/`SockOptKind`/`SockOptValue`, closing
  issue #182 — `SO_REUSEADDR`, `SO_RCVTIMEO`/`SO_SNDTIMEO`, `TCP_NODELAY`
  (`IPPROTO_TCP` level, reusing the already-verified `Protocol::Tcp = 6`
  value), and `SO_ERROR`. Unlike `socket`/`bind`/`listen`/etc., the real
  symbols `setsockopt`/`getsockopt` (no underscore) don't collide with
  this module's `set_sockopt`/`get_sockopt` wrapper names, so no
  `#[link_name]` was needed here. Deliberately no `timeval`-shaped
  option: Windows' `SO_RCVTIMEO`/`SO_SNDTIMEO` take a plain millisecond
  `DWORD` directly, unlike POSIX — documented on `SockOpt::RecvTimeout`/
  `SendTimeout`. Every option this module supports is a 4-byte
  `BOOL`/`DWORD` on the wire, so `set_sockopt`/`get_sockopt` share one
  fixed-size buffer rather than a per-option size. `SOL_SOCKET`/
  `SO_REUSEADDR`/`SO_RCVTIMEO`/`SO_SNDTIMEO`/`SO_ERROR`/`TCP_NODELAY`
  verified via a compiled mingw-w64 probe. Tested each option's
  set-then-get round trip on a freshly created socket, plus `SO_ERROR`
  reporting `0` on a healthy, never-failed socket.

## PR #252 — net: add shutdown
**2026-07-24** · [#252](https://github.com/baileyrd/rusty_win32/pull/252)

- **Added:** `net::shutdown` (`shutdown`) plus `ShutdownHow`
  (`Receive`/`Send`/`Both`), closing issue #181 — half-close a connected
  socket's send and/or receive direction before `close_socket`. Distinct
  from closing the handle outright: after `shutdown(sock,
  ShutdownHow::Send)` the socket can still `recv` any data the peer sent
  before observing the close. Same lowercase-symbol collision as the rest
  of this module's BSD-socket wrappers — bound via
  `#[link_name = "shutdown"]` on a distinctly-named `raw_shutdown`
  extern. `ShutdownHow`'s discriminants (`SD_RECEIVE`/`SD_SEND`/
  `SD_BOTH` = 0/1/2) verified via a compiled mingw-w64 probe. Tested
  end-to-end over a real local TCP connection: after the client calls
  `shutdown(.., Send)`, the accepted socket's `recv` observes a clean
  `Ok(0)` end-of-stream, the same signal `recv` reports for the peer's
  own orderly `closesocket`.

## PR #251 — net: add sendto/recvfrom
**2026-07-24** · [#251](https://github.com/baileyrd/rusty_win32/pull/251)

- **Added:** `net::sendto`/`net::recvfrom` (`sendto`/`recvfrom`),
  closing issue #180 — connectionless datagram I/O, the bare UDP round
  trip. Each call marshals a `sockaddr_in`/`sockaddr_in6` address (via
  the existing `to_sockaddr`/`from_sockaddr` plumbing from PR #246)
  rather than relying on a fixed peer the way `send`/`recv` (PR #250) do
  after `connect`. Same lowercase-symbol collision as the rest of this
  module's BSD-socket wrappers — bound via `#[link_name = "sendto"]`/
  `#[link_name = "recvfrom"]` on distinctly-named `raw_sendto`/
  `raw_recvfrom` externs. Tested end-to-end with two UDP sockets on
  fixed loopback ports (avoiding a dependency on `getsockname`, not yet
  implemented): `sendto` from one, `recvfrom` on the other, verifying
  both the datagram's content and the reported sender address.

## PR #250 — net: add send/recv
**2026-07-24** · [#250](https://github.com/baileyrd/rusty_win32/pull/250)

- **Added:** `net::send`/`net::recv` (`send`/`recv`), closing issue #179
  — byte-slice I/O on a connected socket, thin wrappers matching
  `console::read`/`console::write`'s shape. `recv` reports `Ok(0)` for
  the peer's orderly shutdown, not an error — the TCP analog of
  `ReadFile` reporting end-of-file. Same lowercase-symbol collision as
  `socket`/`bind`/`listen`/`accept`/`connect` — bound via
  `#[link_name = "send"]`/`#[link_name = "recv"]` on distinctly-named
  `raw_send`/`raw_recv` externs. Tested end-to-end over a real local TCP
  connection (server/client both this crate's own sockets, connected via
  PR #249's `connect`/`accept`): sends a message from the client and
  reads it back on the accepted socket, verifying both the byte count
  and content.

## PR #249 — net: add connect
**2026-07-24** · [#249](https://github.com/baileyrd/rusty_win32/pull/249)

- **Added:** `net::connect` (`connect`), closing issue #178 — TCP client
  connect, or fix a UDP socket's default peer for future `send`/`recv`
  calls. Same lowercase-symbol collision as `socket`/`bind`/`listen`/
  `accept` — bound via `#[link_name = "connect"]` on a distinctly-named
  `raw_connect` extern. Tested with a fully self-contained local TCP
  handshake using only this crate's own primitives on both ends
  (`bind`/`listen`/`accept` on one thread, `socket`/`connect` on the
  main thread) — the first `net` test that no longer needs
  `std::net::TcpStream` as a stand-in client, now that this crate has
  its own `connect`.

## PR #248 — net: add accept
**2026-07-24** · [#248](https://github.com/baileyrd/rusty_win32/pull/248)

- **Added:** `net::accept` (`accept`), closing issue #177 — accept one
  incoming TCP connection, returning a new, already-connected socket
  plus the peer's address. `sock` itself stays open and listening
  afterward. This is `from_sockaddr`'s first real (non-test) caller
  (built alongside `to_sockaddr` in PR #246 but unused outside tests
  until now) — its `#[allow(dead_code)]` is removed. Same lowercase-
  symbol collision as `socket`/`bind`/`listen` — bound via
  `#[link_name = "accept"]` on a distinctly-named `raw_accept` extern.
  Tested end-to-end: binds/listens on a fixed loopback port, connects a
  real client via `std::net::TcpStream` on a background thread (this
  crate has no `connect` yet — a later round-2 item), and verifies
  `accept` returns a usable socket and the expected loopback peer
  address.

## PR #247 — net: add listen
**2026-07-24** · [#247](https://github.com/baileyrd/rusty_win32/pull/247)

- **Added:** `net::listen` (`listen`), closing issue #176 — mark a bound
  TCP socket passive/listening, needed before it can accept incoming
  connections. `backlog` is passed through to `listen` unmodified — this
  crate applies no policy to it. The lowercase Win32/BSD-sockets symbol
  `listen` again collides with this crate's own `net::listen` wrapper
  (the same issue PR #245's `socket` and PR #246's `bind` hit) — bound
  via `#[link_name = "listen"]` on a distinctly-named `raw_listen`
  extern. Tested binding a TCP/IPv4 socket to `127.0.0.1:0` (port `0`
  asks Windows for any free ephemeral port) and marking it listening.

## PR #246 — net: add SocketAddr/sockaddr plumbing + bind
**2026-07-24** · [#246](https://github.com/baileyrd/rusty_win32/pull/246)

- **Added:** `SocketAddr` (`V4`/`V6`) plus `to_sockaddr`/`from_sockaddr`
  conversions and the verified `sockaddr_in`/`sockaddr_in6` wire-format
  layouts (16 and 28 bytes, field-by-field verified against mingw-w64's
  own `psdk_inc/_ip_types.h`/`ws2ipdef.h`), closing issue #185 — the
  shared `{ip, port}` address representation every address-taking `net`
  function needs. `ip` octets are stored exactly as they appear on the
  wire (no endian conversion needed); only `port` is byte-swapped to/from
  network byte order internally.
- `net::bind` (`bind`), closing issue #175 — attach a local address/port
  to a socket.
- **Note:** #185 was implemented ahead of its filed order (before #175
  through #184) because issue #175's own literal signature
  (`bind(sock, addr: &SocketAddr)`) already required it — implementing
  #175 in isolation wasn't possible without first building the
  `SocketAddr` plumbing #185 owns. Rather than duplicate that work or
  invent a throwaway stand-in, this PR builds both together and closes
  both issues. `from_sockaddr` has no non-test caller yet (its real
  callers — `accept`/`recvfrom`/`local_addr`/`peer_addr` — are later
  round-2 items), so it carries a documented `#[allow(dead_code)]` until
  those land. The lowercase Win32/BSD-sockets symbol `bind` again
  collides with this crate's own `net::bind` wrapper (the same issue
  PR #245's `socket` hit) — bound via `#[link_name = "bind"]` on a
  distinctly-named `raw_bind` extern, matching that PR's established
  pattern.

## PR #245 — net: add socket/close_socket
**2026-07-24** · [#245](https://github.com/baileyrd/rusty_win32/pull/245)

- **Added:** `net::socket`/`net::close_socket` (`socket`/`closesocket`)
  plus `RawSocket`, `AddressFamily` (`Inet`/`Inet6`), `SocketKind`
  (`Stream`/`Dgram`), and `Protocol` (`Tcp`/`Udp`), closing issue #174 —
  socket lifecycle create/destroy. `RawSocket = usize` (matching
  `std::os::windows::io::RawSocket` and mingw's own `SOCKET` typedef) is
  a distinct handle namespace from `handle::RawHandle`: a `SOCKET` is
  closed via `close_socket`, never `CloseHandle`. The real Win32/BSD-
  sockets symbol is lowercase `socket`, which would otherwise collide
  with this crate's own `net::socket` wrapper function — bound via
  `#[link_name = "socket"]` on a distinctly-named `raw_socket` extern,
  a new pattern for this crate (introduced here since `net`'s BSD-socket
  API surface is the first to use lowercase C symbol names that clash
  with natural snake_case wrapper names). Tested creating and closing
  both TCP/IPv4 and UDP/IPv4 sockets, plus a TCP/IPv6 socket.

## PR #244 — net: add startup/cleanup
**2026-07-24** · [#244](https://github.com/baileyrd/rusty_win32/pull/244)

- **Added:** new `net` module — `net::startup`/`net::cleanup`
  (`WSAStartup`/`WSACleanup`), closing issue #173 — Winsock's own
  load/unload lifecycle, previously excluded by this crate's own
  non-goals, now in scope per explicit round-2 direction. The one
  Winsock primitive with no POSIX/`rusty_libc` analog: every other
  Winsock call is documented undefined behavior before a matching
  `WSAStartup` or after `WSACleanup`. Windows reference-counts nested
  `WSAStartup`/`WSACleanup` pairs internally, so no shared guard/RAII
  type is needed — two plain functions, matching this crate's existing
  no-`Drop`-anywhere convention. `startup` reports failure via its own
  return value directly (never `GetLastError`); `cleanup` uses
  `WSAGetLastError`, a distinct per-thread error slot Winsock keeps
  separately from the ordinary `GetLastError`/`SetLastError` one. The
  `WSADATA` out-parameter (408 bytes on 64-bit, verified field-by-field
  against mingw-w64's own `psdk_inc/_wsadata.h`) is scratch space only,
  never read by this crate.

## PR #243 — service: add dependent_services
**2026-07-24** · [#243](https://github.com/baileyrd/rusty_win32/pull/243)

- **Added:** `service::dependent_services` (`EnumDependentServicesW`)
  plus `DependentService`/`ServiceState`/`SERVICE_ENUMERATE_DEPENDENTS`,
  closing issue #172 — list every service depending on a given one, the
  "will stopping this break something else" check before calling
  `control` with `ServiceControl::Stop`. Reuses `control`'s existing
  `ServiceStatusRaw` (the older, pid-less `SERVICE_STATUS`) rather than
  `enum_services`'s `SERVICE_STATUS_PROCESS`, since
  `EnumDependentServicesW` reports the former. Grows the buffer and
  retries on either `ERROR_MORE_DATA` (this function's own documented
  failure code) or `ERROR_INSUFFICIENT_BUFFER` (treated the same, out of
  caution after PR #242's own buffer-sizing surprise). This completes
  the `service` module's round-2 batch — issues #165 through #172, PRs
  #236-#243. The round-2 parity-loop now moves into the `net` module
  (issues #173-#186).

## PR #242 — service: add display_name/key_name
**2026-07-24** · [#242](https://github.com/baileyrd/rusty_win32/pull/242)

- **Added:** `service::display_name`/`service::key_name`
  (`GetServiceDisplayNameW`/`GetServiceKeyNameW`), closing issue #171 —
  translate between a service's short key name (`"eventlog"`) and its
  human-readable display name (`"Windows Event Log"`). Tested
  round-tripping the well-known "EventLog" service's display name back
  to its key name (case-insensitive, matching Windows' own service-name
  comparison semantics).
- **Fixed:** `display_name`/`key_name` now retry (bounded, up to four
  extra attempts) with a buffer grown past whatever size
  `GetServiceDisplayNameW`/`GetServiceKeyNameW` themselves reported on
  `ERROR_INSUFFICIENT_BUFFER`, instead of trusting that size exactly for
  the very next call — CI caught a real Windows API quirk where the
  reported required size is sometimes still one character short.

## PR #241 — service: add config
**2026-07-24** · [#241](https://github.com/baileyrd/rusty_win32/pull/241)

- **Added:** `service::config` (`QueryServiceConfigW`) plus
  `ServiceConfig`, closing issue #170 — one service's static
  configuration (start type, binary path, display name, load-order
  group, dependencies), for a `systemctl show`-style detail view. Grows
  the buffer on `ERROR_INSUFFICIENT_BUFFER` and retries, needing only
  one retry in practice since `QueryServiceConfigW` reports the exact
  required size up front (unlike PR #237's `enum_services`, which pages
  across multiple calls). `binary_path_name`/`load_order_group`/
  `service_start_name`/`display_name` reuse the existing
  `decode_wide_cstr` helper; `dependencies` decodes `lpDependencies`'s
  `REG_MULTI_SZ`-shaped double-NUL-terminated list into an owned
  `Vec<String>` via a new `decode_wide_multi_sz` helper, the same
  encoding `registry::RegistryValue::MultiSz` already decodes.

## PR #240 — service: add control
**2026-07-24** · [#240](https://github.com/baileyrd/rusty_win32/pull/240)

- **Added:** `service::control` (`ControlService`) plus `ServiceControl`
  (`Stop`/`Pause`/`Continue`/`Interrogate`) and `SERVICE_INTERROGATE`,
  closing issue #169 — send a stop/pause/continue/interrogate control to
  a running service, one `dwControl`-selected call. Deliberately
  discards `ControlService`'s own `lpServiceStatus` out-parameter (kept
  as a private, unexposed `ServiceStatusRaw` FFI mirror of the older,
  pid-less `SERVICE_STATUS` struct) rather than returning it: the
  immediate status only reflects the instant of the call (often still
  `_PENDING`, not settled), so a caller polls PR #238's `status`
  afterward instead — the same poll-don't-block shape
  `job::process_ids` already uses. Tested non-destructively: `Interrogate`
  never changes a service's state, so exercising it against the
  well-known "EventLog" service leaves the machine untouched.

## PR #239 — service: add start
**2026-07-23** · [#239](https://github.com/baileyrd/rusty_win32/pull/239)

- **Added:** `service::start` (`StartServiceW`) plus `SERVICE_START`,
  closing issue #168 — start an already-installed service, the
  zero-argument case only (`lpServiceArgVectors` only matters for
  driver-style services, out of scope). Also adds
  `Win32Error::ERROR_SERVICE_ALREADY_RUNNING` (1056), needed for
  `start`'s error path. Tested non-destructively: calling `start` on the
  well-known "EventLog" service (already running by the time the test
  executes, confirmed via PR #238's `status`) exercises the real
  `StartServiceW` failure path without this crate's own test suite
  actually starting or stopping anything on the CI machine.

## PR #238 — service: add status
**2026-07-23** · [#238](https://github.com/baileyrd/rusty_win32/pull/238)

- **Added:** `service::status` (`QueryServiceStatusEx`,
  `SC_STATUS_PROCESS_INFO`) plus `ServiceStatus` and the seven
  `SERVICE_STOPPED`/`SERVICE_START_PENDING`/`SERVICE_STOP_PENDING`/
  `SERVICE_RUNNING`/`SERVICE_CONTINUE_PENDING`/`SERVICE_PAUSE_PENDING`/
  `SERVICE_PAUSED` state constants, closing issue #167 — one named
  service's live status including its backing process id, superseding
  the older, pid-less `QueryServiceStatus`. `ServiceStatus` carries the
  same fields as PR #237's `ServiceStatusEntry`, minus the two names (a
  caller querying by handle already knows which service it is); both
  reuse the same fixed-size, no-pointers `SERVICE_STATUS_PROCESS` FFI
  mirror, so unlike `enum_services` no growable buffer is needed — its
  exact size is known at compile time. Tested against the well-known
  "EventLog" service (same choice as PRs #236/#237).

## PR #237 — service: add enum_services
**2026-07-23** · [#237](https://github.com/baileyrd/rusty_win32/pull/237)

- **Added:** `service::enum_services` (`EnumServicesStatusExW`) plus
  `ServiceStatusEntry` and `SC_MANAGER_ENUMERATE_SERVICE`/`SERVICE_WIN32`/
  `SERVICE_ACTIVE`/`SERVICE_INACTIVE`/`SERVICE_STATE_ALL`, closing issue
  #166 — list every service known to the SCM with its current status,
  the core of a `systemctl list-units`-equivalent. Pages internally via
  `EnumServicesStatusExW`'s own resume-handle protocol: grows the buffer
  and retries the same page only when nothing at all fit
  (`ERROR_MORE_DATA` with zero entries returned), otherwise processes
  whatever entries came back — including on a partial-page
  `ERROR_MORE_DATA` — and continues until the call finally succeeds.
  `lpServiceName`/`lpDisplayName` point into the same buffer the call
  fills (via a new `decode_wide_cstr` helper handling the
  possibly-unaligned string data), copied out into owned `String`s
  before the buffer drops. Tested against the well-known "EventLog"
  service (PR #236's same choice), present on every Windows edition.

## PR #236 — service: add open_manager/open_service/close
**2026-07-23** · [#236](https://github.com/baileyrd/rusty_win32/pull/236)

- **Added:** new `service` module — `service::open_manager`/
  `service::open_service`/`service::close` (`OpenSCManagerW`/
  `OpenServiceW`/`CloseServiceHandle`) plus `SC_MANAGER_CONNECT`/
  `SERVICE_QUERY_CONFIG`/`SERVICE_QUERY_STATUS`, closing issue #165 —
  the SCM/service handle lifecycle, first piece of a `systemctl`-
  equivalent (list/query/start/stop a named service), previously
  excluded by this crate's own non-goals, now in scope per explicit
  round-2 direction. `SC_HANDLE` is reused as `handle::RawHandle`
  (matching the issue's own literal signature) rather than a distinct
  type — it's ABI-compatible (`DECLARE_HANDLE`-based, exactly like
  `HANDLE` itself) — but its destructor is always `service::close`,
  never `handle`'s own close functions. Tested against the well-known
  "EventLog" service, present on every Windows edition including Server
  Core.

## PR #235 — security: add sd_to_string/string_to_sd
**2026-07-23** · [#235](https://github.com/baileyrd/rusty_win32/pull/235)

- **Added:** `security::sd_to_string`/`security::string_to_sd`
  (`ConvertSecurityDescriptorToStringSecurityDescriptorW`/
  `ConvertStringSecurityDescriptorToSecurityDescriptorW`) plus
  `ConvertedSecurityDescriptor` (freeing via `LocalFree` on `Drop`,
  matching `ConvertedSid`/`BuiltAcl`'s existing pattern), closing issue
  #164 — a debug/snapshot (`icacls /save`-style) SDDL string
  representation of a security descriptor's full permission state.
  `sd_to_string` takes an `info: SecurityInfoFlags` parameter (reusing
  `path_security_info`'s own type) selecting which components to render
  — diverges from the issue's literal `sd_to_string(sd) -> Result`
  signature, since the real Win32 function requires a
  `SECURITY_INFORMATION` selecting owner/group/DACL/SACL. Also adds
  `PathSecurityInfo::raw_security_descriptor`, a new accessor exposing
  the whole self-relative SD block `path_security_info` fetches (needed
  to round-trip test `sd_to_string` against a real file). This completes
  the `security` module's first round-2 batch — issues #154 through
  #164, PRs #225-#235.

## PR #234 — security: add well_known_sid
**2026-07-23** · [#234](https://github.com/baileyrd/rusty_win32/pull/234)

- **Added:** `security::well_known_sid` (`CreateWellKnownSid`) plus
  `WellKnownSidType` (`Everyone`/`LocalSystem`/`BuiltinAdministrators`),
  closing issue #163 — constructs a well-known SID directly, without
  PR #229's `lookup_account_name` name-lookup round trip. Uses the
  query-size-then-allocate idiom (`CreateWellKnownSid` itself reports the
  required buffer size on a first null-buffer call, matching
  `lookup_account_sid`/`lookup_account_name`'s existing pattern) and
  reuses the existing `SidBuf` type. Only the three variants this issue
  named are exposed; `CreateWellKnownSid` supports many more, left out
  until a real need for them shows up. Tested against PR #233's
  `sid_equal` (constructing `Everyone` matches `lookup_account_name`'s own
  resolution of it) and PR #232's `is_valid_sid`.

## PR #233 — security: add sid_equal
**2026-07-23** · [#233](https://github.com/baileyrd/rusty_win32/pull/233)

- **Added:** `security::sid_equal` (`EqualSid`), closing issue #162 —
  byte-correct SID comparison. A naive memory comparison of two `PSID`
  buffers isn't safe: a SID's trailing sub-authority count varies its
  total size, so two equal SIDs aren't guaranteed to be the same number
  of bytes to begin with (and two different-length buffers could still
  collide on a prefix). Tested against PR #232's `copy_sid` (a SID and
  its own copy compare equal) and PR #229's `lookup_account_name`'s
  well-known "Everyone" SID (a real file's owner never equals it).

## PR #232 — security: add sid_length/is_valid_sid/copy_sid
**2026-07-23** · [#232](https://github.com/baileyrd/rusty_win32/pull/232)

- **Added:** `security::sid_length`/`security::is_valid_sid`/
  `security::copy_sid` (`GetLengthSid`/`IsValidSid`/`CopySid`), closing
  issue #161 — sizing, validity-checking, and owned-copying of an opaque
  `PSID`. `copy_sid` sizes the destination buffer via `GetLengthSid`
  itself before calling `CopySid`, and reuses the existing `SidBuf` type
  (from PR #229's `lookup_account_name`) rather than introducing a new
  owned-SID representation — this is the only other place in the module
  that produces one, needed anywhere a SID must outlive the short-lived
  buffer it was originally borrowed from (an owner/ACE SID from
  `path_security_info`/`acl_entries`, both of which only stay valid as
  long as their source does).

## PR #231 — security: add initialize_acl/add_access_allowed_ace/add_access_denied_ace
**2026-07-23** · [#231](https://github.com/baileyrd/rusty_win32/pull/231)

- **Added:** `security::initialize_acl`/`security::add_access_allowed_ace`/
  `security::add_access_denied_ace` (`InitializeAcl`/
  `AddAccessAllowedAce`/`AddAccessDeniedAce`), closing issue #160 — the
  lower-level, per-ACE alternative to PR #227's `build_acl` (which builds
  a whole ACL at once via `SetEntriesInAclW`), useful for a brand-new
  object's initial ACL. `initialize_acl` takes a caller-owned `&mut [u8]`
  buffer and turns it into a fresh, empty `PACL` in place — no alignment
  requirement on the buffer, since this module never reads an `Acl`'s
  fields directly, always through `GetAclInformation`/`GetAce`.
  `add_access_allowed_ace`/`add_access_denied_ace` then append one ACE at
  a time onto that same buffer.

## PR #230 — security: add sid_to_string/string_to_sid
**2026-07-23** · [#230](https://github.com/baileyrd/rusty_win32/pull/230)

- **Added:** `security::sid_to_string`/`security::string_to_sid`
  (`ConvertSidToStringSidW`/`ConvertStringSidToSidW`), closing issue #159 —
  a SID's `S-1-5-...` string form, the fallback `icacls` itself uses when
  a SID can't be resolved to a name at all (orphaned/foreign/deleted
  account) — see PR #229's `lookup_account_sid` for the name-resolving
  path this is a fallback from. `sid_to_string` frees
  `ConvertSidToStringSidW`'s own output buffer via `LocalFree` before
  returning, so the result is an ordinary owned `String` rather than
  another `LocalFree`-on-`Drop` wrapper. `string_to_sid` returns a new
  `ConvertedSid` type (freeing the parsed `PSID` via `LocalFree` on
  `Drop`, matching `BuiltAcl`'s existing pattern) since this crate has no
  `GetLengthSid` yet (a later round-2 item) to copy the SID's bytes into
  an owned `SidBuf` instead. Also added `Win32Error::ERROR_INVALID_SID`
  (`1337`), needed for `string_to_sid`'s malformed-input error path.

## PR #229 — security: add lookup_account_sid/lookup_account_name
**2026-07-23** · [#229](https://github.com/baileyrd/rusty_win32/pull/229)

- **Added:** `security::lookup_account_sid`/`security::lookup_account_name`
  (`LookupAccountSidW`/`LookupAccountNameW`) plus new `AccountName`/
  `SidNameUse`/`SidBuf` types, closing issue #158 — SID↔name resolution.
  `lookup_account_sid` turns an owner/ACE `PSID` into a `"DOMAIN\name"`-
  style display, the way `icacls`/`ls -l` show a human-readable owner
  instead of a raw SID; `lookup_account_name` is the reverse, turning a
  typed account name into the `PSID` `build_trustee_with_sid`/`build_acl`
  need, the way `chown` accepts a username. `SidBuf` is this module's
  only *owned* SID representation — every other `PSID` it touches is a
  borrowed pointer into someone else's memory. Both functions use the
  query-size-then-allocate idiom this crate already uses elsewhere.

## PR #228 — security: add build_trustee_with_sid/build_trustee_with_name
**2026-07-23** · [#228](https://github.com/baileyrd/rusty_win32/pull/228)

- **Added:** `security::build_trustee_with_sid`/
  `security::build_trustee_with_name` (`BuildTrusteeWithSidW`/
  `BuildTrusteeWithNameW`), closing issue #157 — wrap a `PSID` or a
  wide-string name into the `Trustee` shape `build_acl`'s entries need.
  `build_trustee_with_name` takes an already-built, NUL-terminated
  `&mut [u16]` rather than `&str` (diverging from this issue's literal
  signature): `BuildTrusteeWithNameW` doesn't copy the name, it stores
  the pointer it's given directly, so an internally-built temporary
  `Vec<u16>` would leave the result dangling the moment the function
  returned — the caller must own a buffer that actually outlives the
  `Trustee`.
- **Changed:** `Trustee`'s `trustee_form`/`trustee_type`/`name` fields
  are now public (an ordinary FFI-mirror struct, matching
  `ExplicitAccess`'s already-public fields) — needed so a caller can
  adjust `trustee_type` after `build_trustee_with_sid` (which always
  reports `TRUSTEE_IS_UNKNOWN` itself, never inspecting the SID to guess
  a more specific type).
- **Removed:** `Trustee::from_sid`, the crate-internal placeholder
  constructor added alongside `build_acl` (PR #227) before this issue's
  real `BuildTrusteeWithSidW` wrapper existed — superseded now that the
  real primitive covers the same ground; its one caller (`build_acl`'s
  own test) now uses `build_trustee_with_sid` instead.

## PR #227 — security: add build_acl
**2026-07-23** · [#227](https://github.com/baileyrd/rusty_win32/pull/227)

- **Added:** `security::build_acl` (`SetEntriesInAclW`) plus
  `ExplicitAccess`/`Trustee`/`AccessMode`/`TrusteeForm`/`TrusteeType` and
  a new `BuiltAcl` result type, closing issue #156 — build a new ACL
  from an existing one (or from scratch) plus add/replace/remove
  entries, the primitive behind `icacls /grant`/`/deny`. Unlike
  `Acl`/`PSID`, `Trustee`/`ExplicitAccess` are genuinely fixed-size and
  get ordinary, fully-fielded FFI-mirror structs (verified against
  mingw-w64's real `aclapi.h` with a compiled `_Static_assert` probe),
  per `gap-analysis.md`'s design notes for this module. `BuiltAcl` frees
  its `PACL` via `LocalFree` on `Drop`, the same pattern
  `PathSecurityInfo` already uses. `Trustee::from_sid` is the only
  constructor for now — the primitives that would normally build one
  from a name or well-known identity
  (`BuildTrusteeWithSidW`/`CreateWellKnownSid`) are later round-2 items
  this crate doesn't have yet; tested by granting access to a real
  file's own already-obtained owner SID.

## PR #226 — security: add acl_entries
**2026-07-23** · [#226](https://github.com/baileyrd/rusty_win32/pull/226)

- **Added:** `security::acl_entries` (`GetAclInformation` + `GetAce`)
  plus a new `AclEntry`/`AceKind` pair and a fixed-header-only `Acl`
  mirror, closing issue #155 — enumerate a DACL's ACEs one at a time,
  turning an opaque ACL into the human-readable permission list
  `icacls`/`ls -l` displays. `Acl`/`ACE_HEADER`-shaped structs are
  modeled to their fixed header only (per `gap-analysis.md`'s design
  notes for this module), the same "fixed-header-only" treatment
  `fs.rs`'s `ReparseDataBufferSymlinkHeader` already uses for a
  different variable-length Win32 structure — never read directly,
  always through `GetAclInformation`/`GetAce`. Only the two ordinary
  allow/deny ACE kinds are decoded into a `Mask`/`SID`; any other kind
  (audit, object-specific, callback, …) is still reported, as
  `AceKind::Other`, rather than silently dropped from the list.

## PR #225 — security: add path_security_info/set_path_security_info
**2026-07-23** · [#225](https://github.com/baileyrd/rusty_win32/pull/225)

- **Added:** new `security` module — `security::path_security_info`/
  `security::set_path_security_info` (`GetNamedSecurityInfoW`/
  `SetNamedSecurityInfoW`) plus `PathSecurityInfo` and
  `SecurityInfoFlags` (`OWNER_SECURITY_INFORMATION`/
  `GROUP_SECURITY_INFORMATION`/`DACL_SECURITY_INFORMATION`), closing
  issue #154 — the core path → owner `PSID`/DACL `PACL` round trip an
  `icacls`/`ls -l`/`chmod`/`chown`-equivalent is built on. `PSID`/`PACL`
  stay opaque pointers this crate never decodes structurally (per
  `gap-analysis.md`'s design notes, unlike this crate's usual
  `_Static_assert`-verified struct mirrors); `PathSecurityInfo` frees the
  whole `PSECURITY_DESCRIPTOR` block via `LocalFree` on `Drop` — this
  crate's second `Drop` impl (after `volume::FindVolumes`). Tested via a
  self-contained round trip: read a real file's own owner/DACL, then
  re-apply them back onto itself (always permitted without elevated
  privilege), since SID-construction primitives
  (`well_known_sid`/`ConvertStringSidToSidW`) are later round-2 items
  this crate doesn't have yet. First piece of a subsystem previously
  excluded by this crate's own non-goals, now in scope per explicit
  round-2 direction.

## PR #224 — registry: add delete_tree
**2026-07-23** · [#224](https://github.com/baileyrd/rusty_win32/pull/224)

- **Added:** `registry::delete_tree` (`RegDeleteTreeW`), closing issue
  #153 — recursively delete a subkey and everything beneath it (values,
  subkeys, and their own subkeys) in one call, without `delete_key`'s
  leaf-only restriction forcing a hand-rolled enumerate-and-recurse loop.
  This completes the `registry` module's round-2 item list (issues
  #142-#153).

## PR #223 — registry: add flush_key
**2026-07-23** · [#223](https://github.com/baileyrd/rusty_win32/pull/223)

- **Added:** `registry::flush_key` (`RegFlushKey`), closing issue #152 —
  force a key's changes to disk immediately instead of Windows' lazy
  flush, the registry analog of `FlushFileBuffers`. A real durability gap
  for settings writes right before a risky operation (e.g. right before
  terminating the process) — expensive, so documented (matching
  Microsoft's own guidance) as something to call only when durability
  genuinely matters, not as routine practice after every write. This
  completes the `registry` module's round-2 item list (issues
  #142-#152); the next round-2 subsystem is `security`.

## PR #222 — registry: add key_info
**2026-07-23** · [#222](https://github.com/baileyrd/rusty_win32/pull/222)

- **Added:** `registry::key_info` (`RegQueryInfoKeyW`) plus a new
  `KeyInfo` struct, closing issue #151 — subkey/value counts and maximum
  name/data lengths in one call, the "ask how big first" pattern
  `fs::final_path` already uses elsewhere. `enum_values`/`enum_keys`
  already ran this same query internally to pre-size their own buffers;
  this exposes it directly for a caller that wants the counts/lengths
  themselves. Doesn't include the class name (a legacy concept with no
  real modern use) or the security descriptor length (the `security`
  module's territory, not this one's). This completes the `registry`
  module's round-2 item list (issues #142-#151).

## PR #221 — registry: add enum_keys
**2026-07-23** · [#221](https://github.com/baileyrd/rusty_win32/pull/221)

- **Added:** `registry::enum_keys` (`RegEnumKeyExW`) plus a new
  `RegKeyIter` iterator type, closing issue #150 — enumerate a key's
  immediate subkeys as `(String, Timespec)` pairs (name plus last-write
  time), until `ERROR_NO_MORE_ITEMS`. Sizes its name buffer once up front
  via `RegQueryInfoKeyW`'s reported maximum subkey-name length, with the
  same defensive `ERROR_MORE_DATA` grow-and-retry fallback `enum_values`
  uses. The raw `FILETIME` `RegEnumKeyExW` reports is decoded into
  `Timespec` rather than exposed as its own public type — matching this
  crate's "raw FFI struct mirror stays module-private, only the decoded
  value is public" convention (`process::times`, `fs::stat`, …) — the
  filed issue's literal wording named a public `FileTime` return type,
  but this crate already has a real, meaningful timestamp type for
  exactly this purpose and a second, redundant public type wasn't
  warranted.

## PR #220 — registry: add enum_values
**2026-07-23** · [#220](https://github.com/baileyrd/rusty_win32/pull/220)

- **Added:** `registry::enum_values` (`RegEnumValueW`) plus a new
  `RegValueIter` iterator type, closing issue #149 — the value-side
  analog of `fs::read_dir`'s iterator shape, yielding `(String,
  RegistryValue)` pairs until `ERROR_NO_MORE_ITEMS`. Sizes its name/data
  buffers once up front via `RegQueryInfoKeyW`'s reported maximums rather
  than growing from nothing on the first item, with a defensive
  `ERROR_MORE_DATA`-driven grow-and-retry fallback for a value added
  concurrently that exceeds those maximums. Also adds
  `Win32Error::ERROR_NO_MORE_ITEMS` (259), this crate's enumeration
  end-of-sequence code.

## PR #219 — registry: add delete_key
**2026-07-23** · [#219](https://github.com/baileyrd/rusty_win32/pull/219)

- **Added:** `registry::delete_key` (`RegDeleteKeyExW`) plus
  `KEY_WOW64_64KEY`/`KEY_WOW64_32KEY`, closing issue #148 — remove a leaf
  subkey (Windows refuses to delete a key that still has subkeys of its
  own, the same restriction `rmdir` has for a non-empty directory). The
  earlier `create_key`/`set_value`/`delete_value` tests now also clean up
  the subkeys they create, now that `delete_key` exists to do it — the
  registry litter those tests previously left behind (deliberately, since
  no `delete_key` existed yet) is now closed out.

## PR #218 — registry: add delete_value
**2026-07-23** · [#218](https://github.com/baileyrd/rusty_win32/pull/218)

- **Added:** `registry::delete_value` (`RegDeleteValueW`), closing issue
  #147 — remove one named value under an open key, without touching the
  key itself or its other values/subkeys. The earlier `set_value` round
  trip test now also cleans up the values it wrote, now that
  `delete_value` exists to do it (the key itself still can't be removed —
  no `delete_key` yet).

## PR #217 — registry: add set_value
**2026-07-23** · [#217](https://github.com/baileyrd/rusty_win32/pull/217)

- **Added:** `registry::set_value` (`RegSetValueExW`), closing issue
  #146 — the write-side counterpart to `query_value`, encoding a
  `RegistryValue` back into the `dwType`/byte-buffer shape each `REG_*`
  type expects (UTF-16 with a NUL terminator for `Sz`/`ExpandSz`, each
  string NUL-terminated plus a trailing extra NUL for `MultiSz`,
  little-endian bytes for `Dword`/`Qword`, raw bytes as-is for `Binary`).
  Tested via a full set-then-query round trip through every
  `RegistryValue` variant under a per-process-unique `HKEY_CURRENT_USER`
  subkey (uniquified by pid for the same reason `create_key`'s own test
  is — no `delete_key` yet to clean up between this crate's two
  same-VM `cargo test` invocations).

## PR #216 — registry: add query_value
**2026-07-23** · [#216](https://github.com/baileyrd/rusty_win32/pull/216)

- **Added:** `registry::query_value` (`RegQueryValueExW`) plus a new
  `RegistryValue` enum (`None`/`Sz`/`ExpandSz`/`Dword`/`Qword`/`Binary`/
  `MultiSz`), closing issue #145 — reads a value's data, decoded by its
  real `dwType` rather than handed back as a raw byte blob plus a
  separate type code. Uses the query-size-then-allocate idiom this crate
  already uses elsewhere (`path::search_path`, `fs::final_path`): a first
  call with a null data pointer reports the exact required size and type,
  then a second call reads the data into a correctly-sized buffer.
  `REG_SZ`/`REG_EXPAND_SZ`/`REG_BINARY`/`REG_DWORD`/`REG_MULTI_SZ`/
  `REG_QWORD` verified against mingw-w64's real `winnt.h` macros.

## PR #215 — registry: add create_key
**2026-07-23** · [#215](https://github.com/baileyrd/rusty_win32/pull/215)

- **Added:** `registry::create_key` (`RegCreateKeyExW`) plus
  `KeyDisposition` (`CreatedNewKey`/`OpenedExistingKey`), closing issue
  #144 — open-or-create in one call, reporting via the returned
  disposition which one happened; an idempotent "ensure this key exists"
  a caller would otherwise build by falling back from `open_key`'s
  `ERROR_FILE_NOT_FOUND`. `REG_CREATED_NEW_KEY`/`REG_OPENED_EXISTING_KEY`/
  `REG_OPTION_NON_VOLATILE` verified against mingw-w64's real `winnt.h`
  macros.
- **Fixed:** a CI-caught real environmental fact in this PR's own new
  test: this crate's CI job runs `cargo test` twice (no-default-features,
  then `--features std`) on the *same* Windows VM, and unlike this
  crate's temp-file-backed tests, there was no `delete_key` yet to clean
  up the registry subkey the first invocation created — so the second
  invocation's own instance of the test found the subkey already existing
  and failed asserting `CreatedNewKey`. Fixed by uniquifying the subkey
  name with the test process's own pid, guaranteeing a fresh key every
  invocation regardless of what an earlier one left behind.

## PR #214 — registry: add open_key/close_key
**2026-07-23** · [#214](https://github.com/baileyrd/rusty_win32/pull/214)

- **Added:** `registry::open_key`/`registry::close_key` (`RegOpenKeyExW`/
  `RegCloseKey`), closing issue #143 — open a subkey of a predefined root
  (or another already-open key), and close it again. `RegOpenKeyExW`
  reports failure via its own `LSTATUS` return value directly rather than
  `GetLastError`, so this maps straight through `Win32Error::from_raw`
  instead of `Win32Error::last()`. Also adds `KEY_READ`/`KEY_WRITE`/
  `KEY_ALL_ACCESS`/`KEY_QUERY_VALUE` REGSAM access-mask constants
  (`access: u32`'s bitmask, needed by `open_key` and every later registry
  call taking one), verified against mingw-w64's real `winnt.h` macros
  with a compiled `_Static_assert` probe.

## PR #213 — registry: add HKey type + predefined root keys
**2026-07-23** · [#213](https://github.com/baileyrd/rusty_win32/pull/213)

- **Added:** new `registry` module — `HKey` (`*mut c_void`, distinct from
  `handle::RawHandle` since a registry key closes via `RegCloseKey`, not
  `CloseHandle`) plus the five predefined root keys
  (`HKEY_CLASSES_ROOT`/`HKEY_CURRENT_USER`/`HKEY_LOCAL_MACHINE`/
  `HKEY_USERS`/`HKEY_CURRENT_CONFIG`), closing issue #142. These are
  sign-extended 64-bit sentinel pointer values (Windows defines each as a
  32-bit signed `LONG` widened to pointer size — `0x80000000` as `LONG` is
  negative, so the real bit pattern is `0xFFFFFFFF80000000`, not
  `0x0000000080000000`) — verified against mingw-w64's own `winreg.h`
  macros with a compiled `_Static_assert` probe. First piece of a brand-new
  subsystem previously excluded by this crate's own non-goals, now in
  scope per explicit round-2 direction (`gap-analysis.md`); the actual
  open/query/set/delete surface is follow-up work.

## PR #212 — console: add screen-buffer/window write side (GetLargestConsoleWindowSize/SetConsoleScreenBufferSize/SetConsoleWindowInfo)
**2026-07-23** · [#212](https://github.com/baileyrd/rusty_win32/pull/212)

- **Added:** `console::largest_window_size`/`console::set_screen_buffer_size`/
  `console::set_window_info` (`GetLargestConsoleWindowSize`/
  `SetConsoleScreenBufferSize`/`SetConsoleWindowInfo`), closing issue #141 —
  the write side of console geometry, complementing the existing read-only
  `window_size`. `Coord`/`SmallRect` (previously private, backing
  `set_cursor_position`/`window_size` internally) are now public types
  too, since these three functions' signatures need them. Another round-2
  "weak/no clear consumer" item (`gap-analysis.md`); no current `rush`
  feature asks for this.
- **Fixed:** a CI-caught real Windows behavior in this PR's own new tests:
  resizing the screen buffer (or even a zero-delta `SetConsoleWindowInfo`
  call) queues a real `WINDOW_BUFFER_SIZE_EVENT` into the console's shared
  input buffer — not a flake, a documented side effect neither new test
  accounted for. This left spurious "ready" input queued for whichever
  test ran next alphabetically
  (`wait_readable_times_out_with_no_pending_input`), which failed
  expecting no input to be pending. Fixed by flushing the input buffer at
  the end of both new tests.

## PR #211 — console: add alloc/free/attach (AllocConsole/FreeConsole/AttachConsole)
**2026-07-23** · [#211](https://github.com/baileyrd/rusty_win32/pull/211)

- **Added:** `console::alloc`/`console::free`/`console::attach`
  (`AllocConsole`/`FreeConsole`/`AttachConsole`), closing issue #140 — lets a
  GUI-subsystem process acquire a console on demand, release it, or attach
  to another process's (`attach(None)` maps to `ATTACH_PARENT_PROCESS`).
  This crate's own test helper (`ensure_console_stdin`) already used
  `AllocConsole` internally; it now calls through the public `alloc()`
  instead of a private duplicate extern. Another round-2 "weak/no clear
  consumer" item (`gap-analysis.md`); no current `rush` feature asks for
  this.
- **Fixed:** a CI-caught bug in this PR's own new test for `attach` — it
  called `free()` on the shared test-process console, then attempted to
  `attach()` to a child process's inherited console, which failed
  (`Win32Error(31)`) in this hosting environment. Because the test never
  restored a console on that failure path, every later test run
  afterward (alphabetically) inherited a console-less process, breaking
  an unrelated, pre-existing test
  (`ctrl_c_event_cannot_be_scoped_to_a_process_group`) that implicitly
  depends on one being attached. Replaced with a non-destructive test of
  `attach`'s documented error path instead (`AttachConsole` rejects the
  call outright when the caller already has a console) — this never
  detaches the shared console, so it can't leak a broken state into
  later tests.

## PR #210 — console: add window_handle (GetConsoleWindow)
**2026-07-23** · [#210](https://github.com/baileyrd/rusty_win32/pull/210)

- **Added:** `console::window_handle` (`GetConsoleWindow`), closing issue
  #139 — the `HWND` of the console window attached to the calling
  process, if any (`None` for a headless/service process). Manipulating
  the window itself via ordinary `user32` window APIs is out of this
  crate's scope beyond returning the handle. Another round-2 "weak/no
  clear consumer" item (`gap-analysis.md`); no current `rush` feature
  asks for this.
- **Fixed:** a genuine (not flaky) test assumption caught by CI: the
  original test asserted `GetConsoleWindow` always returns a real `HWND`
  once a console exists, but this crate's own `windows-latest` CI runner
  is itself a real, deterministic counterexample — a console can exist
  and work fine for I/O with no window attached (e.g. a non-interactive
  CI session). Rewrote the test to check repeated-call consistency
  instead of asserting `Some`.

## PR #209 — console: add process_list (GetConsoleProcessList)
**2026-07-23** · [#209](https://github.com/baileyrd/rusty_win32/pull/209)

- **Fixed:** a CI-caught race in
  `process::tests::thread_times_reports_plausible_creation_and_exit_timestamps`
  (added in PR #193) — the same shape as the earlier `thread_exit_code`
  fix (PR #198): the process handle becoming signaled doesn't guarantee
  `GetThreadTimes`' `exit` field is already populated for the thread
  itself. Fixed by opening the thread handle with `SYNCHRONIZE` too and
  explicitly waiting on it before reading the times. Unrelated to this
  PR's own diff; also unrelated to a separate CI infra hang on the same
  PR (an `apt-get install mingw-w64` step stalled for ~19 min — resolved
  by cancelling and rerunning, no code change needed for that one).
- **Added:** `console::process_list` (`GetConsoleProcessList`), closing
  issue #138 — the pids of every process currently attached to the
  calling process's console, e.g. for an "is anything else still
  attached to this console" check. Another round-2 "weak/no clear
  consumer" item (`gap-analysis.md`); no current `rush` feature asks for
  this.

## PR #208 — process: add exception-handler hooks (AddVectoredExceptionHandler/SetUnhandledExceptionFilter)
**2026-07-23** · [#208](https://github.com/baileyrd/rusty_win32/pull/208)

- **Added:** `process::add_vectored_exception_handler`/
  `remove_vectored_exception_handler` (`AddVectoredExceptionHandler`/
  `RemoveVectoredExceptionHandler`) and
  `process::set_unhandled_exception_filter` (`SetUnhandledExceptionFilter`)
  plus `ExceptionPointers`/`VectoredExceptionHandler`/
  `TopLevelExceptionFilter`/`EXCEPTION_CONTINUE_EXECUTION`/
  `EXCEPTION_CONTINUE_SEARCH`/`EXCEPTION_EXECUTE_HANDLER`, closing issue
  #137 — structured-exception-handling hooks, the closest Windows analog
  to installing a Unix `SIGSEGV`/`SIGABRT` handler. `EXCEPTION_RECORD`/
  `CONTEXT` aren't decoded (a variable-length trailing array and a large,
  architecture-specific register dump respectively) — only the two raw
  pointers `EXCEPTION_POINTERS` itself carries are exposed. Tested via
  `RaiseException` with a custom application-defined exception code — a
  safe, deterministic way to exercise a handler without a real CPU fault.
  Another round-2 "weak/no clear consumer" item (`gap-analysis.md`); no
  current `rush` feature asks for this.

## PR #207 — process: add logical_processor_information (GetLogicalProcessorInformation)
**2026-07-23** · [#207](https://github.com/baileyrd/rusty_win32/pull/207)

- **Added:** `process::logical_processor_information`
  (`GetLogicalProcessorInformation`) plus `LogicalProcessorInformation`/
  `ProcessorRelationship`, closing issue #136 — detailed CPU topology
  (cores/NUMA nodes/cache) beyond `process::logical_processor_count`'s
  single number, using the query-size-then-allocate pattern this crate
  already uses elsewhere. Only `processor_mask`/`relationship` are
  exposed (not cache/NUMA-specific fields) — a future consumer needing
  those can extend it. Another round-2 "weak/no clear consumer" item
  (`gap-analysis.md`); no current `rush` feature asks for this.

## PR #206 — path: add system_directory/windows_directory (GetSystemDirectoryW/GetWindowsDirectoryW)
**2026-07-23** · [#206](https://github.com/baileyrd/rusty_win32/pull/206)

- **Added:** `path::system_directory`/`path::windows_directory`
  (`GetSystemDirectoryW`/`GetWindowsDirectoryW`), closing issue #135 —
  standard well-known-location primitives (`C:\Windows\System32`/
  `C:\Windows`), the Windows analog of resolving `/usr/bin`. Another
  round-2 "weak/no clear consumer" item (`gap-analysis.md`); no current
  `rush` feature asks for this.

## PR #205 — process: add tick_count (GetTickCount64)
**2026-07-23** · [#205](https://github.com/baileyrd/rusty_win32/pull/205)

- **Added:** `process::tick_count` (`GetTickCount64`), closing issue #134
  — milliseconds elapsed since the system started, a coarser, simpler
  monotonic counter alongside `time::now_monotonic`'s
  `QueryPerformanceCounter`-backed high-resolution one. Another round-2
  "weak/no clear consumer" item (`gap-analysis.md`); no current `rush`
  feature asks for this.

## PR #204 — job: add open_by_name (OpenJobObjectW)
**2026-07-23** · [#204](https://github.com/baileyrd/rusty_win32/pull/204)

- **Added:** `job::open_by_name` (`OpenJobObjectW`) plus
  `JOB_OBJECT_ALL_ACCESS`, closing issue #133 — the reverse direction of
  `job::create`, which only ever makes anonymous jobs. Another round-2
  "weak/no clear consumer" item (`gap-analysis.md`); no current `rush`
  feature asks for this.

## PR #203 — pipe: add transact/call (TransactNamedPipe/CallNamedPipeW)
**2026-07-23** · [#203](https://github.com/baileyrd/rusty_win32/pull/203)

- **Added:** `pipe::transact`/`pipe::call` (`TransactNamedPipe`/
  `CallNamedPipeW`), closing issue #132 — one-shot message-mode pipe
  transactions (write-then-read in a single call) for a simple
  request-response protocol; `call` additionally combines
  `wait_for_server`/`open_client`/`transact`/close for a caller that only
  needs one round trip. Another round-2 "weak/no clear consumer" item
  (`gap-analysis.md`); no current `rush` feature asks for this.
- **Fixed:** a real CI hang (not a fast failure) in this PR's own new
  tests: `transact`/`call`'s pipe was created with `PIPE_READMODE_BYTE`
  even though `PIPE_TYPE_MESSAGE` was set, and `TransactNamedPipe`/
  `CallNamedPipeW` are fully synchronous with no timeout — a read/write
  mode mismatch left the call blocked forever instead of erroring.
  Switched both new tests' pipes to `PIPE_READMODE_MESSAGE`. Caught by an
  abnormally long `windows-latest` CI run (~25 min vs. this workflow's
  usual ~1 min), cancelled and re-run after the fix.
- **Fixed:** the same two tests still hung a second `windows-latest` run
  after the fix above (~21 min), confirming the deadlock risk wasn't
  fully understood. Rather than guess a third specific cause, both tests
  now run their server/client halves on independent threads
  communicating over a channel with a 10-second `recv_timeout`, instead
  of a plain `.join()` with no bound — a real deadlock now fails the test
  in ~10s with a clear message instead of hanging the whole CI job again.
- **Fixed:** the watchdog above then reported the real root cause in
  ~11s — `TransactNamedPipe` failed with a genuine `ERROR_BAD_PIPE`
  (230), not a hang. The `call` test passed, isolating the bug to
  `transact`: a freshly-opened client handle defaults to *byte* read
  mode regardless of the server's creation-time `dwPipeMode` —
  `TransactNamedPipe` specifically requires the *calling handle itself*
  to be in message read mode. Fixed by calling `set_pipe_mode` on the
  client (switching it to `PIPE_READMODE_MESSAGE`) before `transact`.

## PR #202 — pipe: add pipe_info (GetNamedPipeInfo)
**2026-07-23** · [#202](https://github.com/baileyrd/rusty_win32/pull/202)

- **Added:** `pipe::pipe_info` (`GetNamedPipeInfo`) plus a new `PipeInfo`
  struct, closing issue #131 — reads back a named pipe's own type/mode/
  buffer-size configuration, the read-side counterpart to
  `pipe::create_server`'s creation-time parameters. Works on either a
  server handle or a client handle. Another round-2 "weak/no clear
  consumer" item (`gap-analysis.md`); no current `rush` feature asks for
  this.

## PR #201 — handle: add same_object (CompareObjectHandles)
**2026-07-23** · [#201](https://github.com/baileyrd/rusty_win32/pull/201)

- **Added:** `handle::same_object` (`CompareObjectHandles`), closing issue
  #130 — the documented-correct way to ask Windows whether two handle
  values refer to the same kernel object, since comparing raw handle
  values isn't guaranteed reliable (values get reused, and `duplicate`
  can validly produce a second value for the same object). Another
  round-2 "weak/no clear consumer" item (`gap-analysis.md`); no current
  `rush` feature asks for this.
- **Fixed:** a real CI-caught linker failure (`LNK2019: unresolved
  external symbol __imp_CompareObjectHandles`) on this crate's own
  `windows-latest` runner: some Windows SDK versions' `kernel32.lib`
  import library omits a static stub for this symbol despite it being a
  real, always-present `kernel32.dll` export. Fixed by resolving it via
  `GetProcAddress` at call time instead of this crate's usual static
  `#[link]` import — new territory for this crate, but scoped to this one
  function. `same_object`'s signature changed to `Result<bool,
  Win32Error>` to report the (practically unreachable) case where the
  lookup itself fails.
- **Fixed:** a second CI-caught issue in the same run, once the link
  failure above was fixed: `GetProcAddress` against `kernel32.dll` alone
  reported `ERROR_PROC_NOT_FOUND` on this runner too — `kernel32.dll`
  doesn't always forward this symbol by name in a way `GetProcAddress`
  resolves, even though the function genuinely exists (implemented in
  `KernelBase.dll`). Fixed by trying `KernelBase.dll` as a fallback module
  if the `kernel32.dll` lookup fails.

## PR #200 — fs: add compressed_file_size (GetCompressedFileSizeW)
**2026-07-23** · [#200](https://github.com/baileyrd/rusty_win32/pull/200)

- **Added:** `fs::compressed_file_size` (`GetCompressedFileSizeW`), closing
  issue #129 — the on-disk (compressed) size of a file vs. `fs::stat`'s
  logical size, meaningful only for an NTFS-compressed file. Disambiguates
  the call's `INVALID_FILE_SIZE` sentinel from a legitimate all-ones
  low-order size via `GetLastError()`, per its documented contract.
  Another round-2 "weak/no clear consumer" item (`gap-analysis.md`); no
  current `rush` feature asks for this.

## PR #199 — volume: add volume_path_name (GetVolumePathNameW)
**2026-07-23** · [#199](https://github.com/baileyrd/rusty_win32/pull/199)

- **Added:** `volume::volume_path_name` (`GetVolumePathNameW`), closing
  issue #128 — maps an arbitrary file/directory path to the root path of
  the volume it's on, the reverse direction of `volume_information`/
  `disk_free_space`'s own root-path parameter. Another round-2 "weak/no
  clear consumer" item (`gap-analysis.md`); no current `rush` feature asks
  for this.

## PR #198 — volume: add find_volumes (FindFirstVolumeW/FindNextVolumeW/FindVolumeClose)
**2026-07-23** · [#198](https://github.com/baileyrd/rusty_win32/pull/198)

- **Fixed:** a CI-caught race in
  `process::tests::thread_exit_code_reports_still_active_then_the_real_code`
  (added in PR #192): the process handle becoming signaled after
  `TerminateProcess` doesn't guarantee `GetExitCodeThread` already
  reflects the thread's final exit code — Windows doesn't document those
  two transitions as atomic with each other. Fixed by opening the thread
  handle with `SYNCHRONIZE` too and explicitly waiting on it (via
  `handle::wait_single_ex`, PR #195) before reading the exit code.
- **Fixed:** a second CI-caught race, in the same run, in
  `job::tests::accounting_reports_process_counts_after_a_process_exits`
  (added in PR #38): the same shape of issue — the process handle becoming
  signaled doesn't guarantee the job object's own `active_processes`
  bookkeeping has already been decremented. Fixed with a short bounded
  poll (up to 1s) instead of asserting on the very first read.
- **Added:** `volume::find_volumes` (`FindFirstVolumeW`/`FindNextVolumeW`/
  `FindVolumeClose`), closing issue #127 — enumerates every volume by its
  stable GUID path (`\\?\Volume{GUID}\`), independent of drive-letter
  assignment, unlike `volume::logical_drives`. Mirrors `fs::read_dir`'s
  `ReadDir` iterator shape (`FindClose`-on-drop). Another round-2 "weak/no
  clear consumer" item (`gap-analysis.md`); no current `rush` feature asks
  for this.

## PR #197 — handle: add signal_and_wait (SignalObjectAndWait)
**2026-07-23** · [#197](https://github.com/baileyrd/rusty_win32/pull/197)

- **Added:** `handle::signal_and_wait` (`SignalObjectAndWait`), closing
  issue #126 — atomically signals one synchronization object (mutex,
  semaphore, or event) and waits on another, avoiding the race a caller
  would otherwise accept making two separate calls. Reuses the `WaitResult`
  type `wait_single_ex`/`wait_multiple_ex` (issue #124) introduced. Another
  round-2 "weak/no clear consumer" item (`gap-analysis.md`); no current
  `rush` feature asks for this.

## PR #196 — process: add sleep_ms_ex (SleepEx)
**2026-07-23** · [#196](https://github.com/baileyrd/rusty_win32/pull/196)

- **Added:** `process::sleep_ms_ex` (`SleepEx`), closing issue #125 — the
  alertable-sleep variant of `process::sleep_ms` (`Sleep`), reporting
  `WAIT_IO_COMPLETION` if an APC woke the sleep early rather than the full
  duration elapsing. Another round-2 "weak/no clear consumer" item
  (`gap-analysis.md`); no current `rush` feature uses APCs.

## PR #195 — handle: add wait_single_ex/wait_multiple_ex (WaitForSingleObjectEx/WaitForMultipleObjectsEx)
**2026-07-23** · [#195](https://github.com/baileyrd/rusty_win32/pull/195)

- **Added:** `handle::wait_single_ex`/`handle::wait_multiple_ex` plus a new
  `WaitResult` enum (`Signaled`/`Abandoned`/`TimedOut`/`IoCompletion`),
  closing issue #124 — alertable-wait variants of the plain waits already
  used throughout this crate (`console::wait_readable`,
  `process::wait`/`wait_any`), adding `alertable` for APC wakeups. Unlike
  `process::wait_any` (scoped to process handles, which are never
  abandoned), this generic pair also reports `WaitResult::Abandoned` for a
  mutex whose owner terminated without releasing it. Another round-2
  "weak/no clear consumer" item (`gap-analysis.md`); no current `rush`
  feature uses APCs.

## PR #194 — handle: add create_semaphore/release_semaphore (CreateSemaphoreW/ReleaseSemaphore)
**2026-07-23** · [#194](https://github.com/baileyrd/rusty_win32/pull/194)

- **Added:** `handle::create_semaphore`/`handle::release_semaphore`
  (`CreateSemaphoreW`/`ReleaseSemaphore`), closing issue #123 — a counting
  semaphore, alongside the already-wrapped mutex (`handle::create_mutex`)
  the other standard Win32 synchronization primitive. Acquiring reuses
  this crate's existing `WaitForSingleObject`-shaped wait primitives
  (`console::wait_readable`), the same pattern `create_mutex` already
  established. Another round-2 "weak/no clear consumer" item
  (`gap-analysis.md`); no current `rush` feature asks for this.

## PR #193 — process: add thread_times (GetThreadTimes)
**2026-07-23** · [#193](https://github.com/baileyrd/rusty_win32/pull/193)

- **Added:** `process::thread_times` (`GetThreadTimes`), closing issue #122
  — the thread-level counterpart to `process::times` (`GetProcessTimes`),
  reusing the same `FileTime`-to-`Timespec` conversions. Another round-2
  "weak/no clear consumer" item (`gap-analysis.md`); no current `rush`
  feature asks for this.

## PR #192 — process: add thread_exit_code (GetExitCodeThread)
**2026-07-23** · [#192](https://github.com/baileyrd/rusty_win32/pull/192)

- **Added:** `process::thread_exit_code` (`GetExitCodeThread`) plus
  `THREAD_QUERY_INFORMATION`, closing issue #121 — the thread-level
  counterpart to `wait`'s process exit code (`GetExitCodeProcess`). Reports
  Windows' own `STILL_ACTIVE` (`259`) sentinel for a not-yet-exited thread
  rather than inventing a distinct error. Another round-2 "weak/no clear
  consumer" item (`gap-analysis.md`); no current `rush` feature asks for
  this.

## PR #191 — process: add affinity/set_affinity (GetProcessAffinityMask/SetProcessAffinityMask)
**2026-07-23** · [#191](https://github.com/baileyrd/rusty_win32/pull/191)

- **Added:** `process::affinity`/`process::set_affinity`
  (`GetProcessAffinityMask`/`SetProcessAffinityMask`), closing issue #120 —
  the first of the round-2 "weak/no clear consumer" items added per
  explicit direction (`gap-analysis.md`), with the usual "needs a `rush`/
  `rusty_lines` consumer" gate dropped for this round. The Windows analog
  of `sched_getaffinity`/`taskset`. No current `rush` feature asks for
  this; filed for Win32 parity/completeness.

## PR #118 — fs: add lock_file/unlock_file (LockFileEx/UnlockFileEx)
**2026-07-23** · [#118](https://github.com/baileyrd/rusty_win32/pull/118)

- **Added:** `fs::lock_file`/`fs::unlock_file` (`LockFileEx`/`UnlockFileEx`),
  closing issue #85 — the last of the 32 issues filed from the
  parity-loop's Win32 coverage sweep (`gap-analysis.md`). The Windows
  analog of `flock` at the file level, an alternative to
  `handle::create_mutex`'s named-mutex option for the same
  shared-history-file-locking use case. Reuses the same `OVERLAPPED`
  shape `watch.rs` already models (duplicated locally per this crate's
  per-module convention), since `LockFileEx`/`UnlockFileEx` require a
  non-NULL `OVERLAPPED` even for an ordinary synchronous handle, using it
  only to carry the 64-bit lock offset. No current `rush` feature asks
  for this; filed for tracking.

## PR #117 — handle: add create_mutex/release_mutex (CreateMutexW/ReleaseMutex)
**2026-07-23** · [#117](https://github.com/baileyrd/rusty_win32/pull/117)

- **Added:** `handle::create_mutex`/`handle::release_mutex`
  (`CreateMutexW`/`ReleaseMutex`), closing issue #84 from the parity-loop
  sweep — the Windows analog of `flock`'s cross-process locking, as a
  standalone kernel object rather than a file-descriptor operation.
  Acquiring an existing mutex is already covered by this crate's
  `WaitForSingleObject`-shaped wait primitives (`console::wait_readable`
  generalizes to any waitable handle, not just console ones) once a handle
  is in hand — no new wait wrapper needed. No current `rush` feature asks
  for this; filed for tracking (a plausible use case is guarding
  concurrent writes to a shared history file from multiple shell
  instances).

## PR #116 — pipe: add set_pipe_mode (SetNamedPipeHandleState)
**2026-07-23** · [#116](https://github.com/baileyrd/rusty_win32/pull/116)

- **Added:** `pipe::set_pipe_mode` (`SetNamedPipeHandleState`), closing
  issue #83 from the parity-loop sweep — exposes the raw
  `PIPE_NOWAIT`/`PIPE_READMODE_*` bits `pipe.rs` already defines, covering
  two things at once: switching between byte/message read mode after
  creation, and `PIPE_NOWAIT`, the named-pipe equivalent of the
  non-blocking check `handle::pipe_bytes_available` (`PeekNamedPipe`)
  already gives anonymous pipes.

## PR #115 — process: add list_threads/open_thread/suspend_thread (Thread32First/Thread32Next/OpenThread/SuspendThread)
**2026-07-23** · [#115](https://github.com/baileyrd/rusty_win32/pull/115)

- **Added:** `process::list_threads` (`CreateToolhelp32Snapshot`
  `TH32CS_SNAPTHREAD`/`Thread32First`/`Thread32Next`),
  `process::open_thread` (`OpenThread`), and `process::suspend_thread`
  (`SuspendThread`) plus `THREAD_SUSPEND_RESUME` — closing issue #82 from
  the parity-loop sweep. Windows has no process-wide stop primitive the
  way Unix `SIGSTOP` does; pausing every thread a process owns
  individually is the closest equivalent, the missing "pause it instead"
  counterpart to `job::terminate`'s "kill a whole job." `resume` (already
  wrapped for `spawn_suspended`'s own use) is the `SIGCONT` half — no new
  resume wrapper needed. `THREADENTRY32`'s layout independently verified
  via a compiled mingw-w64 C probe. Filed for a future `bg`/`fg`/Ctrl-Z-
  style feature currently out of `rush`'s scope, not to enable it now.

## PR #114 — process: add priority_class/set_priority_class (GetPriorityClass/SetPriorityClass)
**2026-07-23** · [#114](https://github.com/baileyrd/rusty_win32/pull/114)

- **Added:** `process::priority_class`/`process::set_priority_class`
  (`GetPriorityClass`/`SetPriorityClass`) plus the `*_PRIORITY_CLASS`
  constants (`IDLE`/`BELOW_NORMAL`/`NORMAL`/`ABOVE_NORMAL`/`HIGH`/
  `REALTIME`), closing issue #81 from the parity-loop sweep — the Windows
  analog of `nice`/`renice`. No current `rush` feature asks for this, but
  it's the natural primitive if a `nice`/`renice`-style feature is ever
  added.

## PR #113 — path: add temp_path/temp_file_name (GetTempPathW/GetTempFileNameW)
**2026-07-23** · [#113](https://github.com/baileyrd/rusty_win32/pull/113)

- **Added:** `path::temp_path`/`path::temp_file_name`
  (`GetTempPathW`/`GetTempFileNameW`), closing issue #80 from the
  parity-loop sweep — needed for heredoc scratch files or a `mktemp`
  builtin. `temp_file_name` documents a real Windows quirk rather than
  working around it silently: `GetTempFileNameW` also *creates* the
  (empty) file as a side effect, unlike a POSIX `mktemp`-style name
  generator, which only reserves a name.

## PR #112 — fs: add create_hard_link (CreateHardLinkW)
**2026-07-23** · [#112](https://github.com/baileyrd/rusty_win32/pull/112)

- **Added:** `fs::create_hard_link` (`CreateHardLinkW`), closing issue #79
  from the parity-loop sweep — `ln`'s (without `-s`) Windows counterpart,
  the non-symbolic counterpart to `create_symlink` (issue #18). Both paths
  must be on the same volume and `target_path` must already exist, a
  documented `CreateHardLinkW` restriction this wrapper doesn't check
  itself.

## PR #111 — fs: add create_directory/remove_directory (CreateDirectoryW/RemoveDirectoryW)
**2026-07-23** · [#111](https://github.com/baileyrd/rusty_win32/pull/111)

- **Added:** `fs::create_directory`/`fs::remove_directory`
  (`CreateDirectoryW`/`RemoveDirectoryW`), closing issue #78 from the
  parity-loop sweep — the primitives behind `mkdir`/`rmdir` builtins.
  `create_directory` only creates the final path component (no `mkdir -p`
  behavior); `remove_directory` requires an empty directory (no `rm -rf`
  recursion). Filed for tracking, not urgency: `rush` has no `mkdir`/`rmdir`
  builtins today.

## PR #110 — fs: add delete_file (DeleteFileW)
**2026-07-23** · [#110](https://github.com/baileyrd/rusty_win32/pull/110)

- **Added:** `fs::delete_file` (`DeleteFileW`), closing issue #77 from the
  parity-loop sweep — the primitive behind an `rm` builtin. Only removes
  files, not directories, matching `DeleteFileW`'s own scope. Filed for
  tracking, not urgency: `rush` has no `rm` builtin today.

## PR #109 — fs: add move_file (MoveFileExW)
**2026-07-23** · [#109](https://github.com/baileyrd/rusty_win32/pull/109)

- **Added:** `fs::move_file` (`MoveFileExW`) plus the
  `MOVEFILE_REPLACE_EXISTING`/`MOVEFILE_COPY_ALLOWED` flag constants,
  closing issue #76 from the parity-loop sweep — the primitive behind an
  `mv` builtin. `MOVEFILE_COPY_ALLOWED` also covers cross-volume moves,
  which `std::fs::rename` doesn't on Windows. Filed for tracking, not
  urgency: `rush` has no `mv` builtin today.

## PR #108 — fs: add copy_file (CopyFileW)
**2026-07-23** · [#108](https://github.com/baileyrd/rusty_win32/pull/108)

- **Added:** `fs::copy_file` (`CopyFileW`), closing issue #75 from the
  parity-loop sweep — the primitive behind a `cp` builtin. `fail_if_exists`
  refuses to overwrite an already-existing destination
  ([`Win32Error::ERROR_FILE_EXISTS`]) rather than this crate deciding that
  policy itself. Filed for tracking, not urgency: `rush` has no `cp`
  builtin today (likely uses `std::fs::copy`), so this is a lateral/optional
  addition unless a lower-level primitive is ever wanted.

## PR #107 — process: add set_error_mode (SetErrorMode)
**2026-07-23** · [#107](https://github.com/baileyrd/rusty_win32/pull/107)

- **Added:** `process::set_error_mode` (`SetErrorMode`) plus the
  `SEM_FAILCRITICALERRORS`/`SEM_NOOPENFILEERRORBOX` constants, closing
  issue #74 from the parity-loop sweep. Without
  `SEM_FAILCRITICALERRORS`, a hardware/media error (e.g. an empty
  removable drive, a network path that's gone away) pops a blocking GUI
  dialog that freezes the whole process — including a non-interactive
  script run with no one there to click it. A real robustness gap worth
  closing early in a shell's own startup path, not just a nice-to-have.

## PR #106 — process: add memory_status (GlobalMemoryStatusEx)
**2026-07-23** · [#106](https://github.com/baileyrd/rusty_win32/pull/106)

- **Added:** `process::memory_status` (`GlobalMemoryStatusEx`), returning a
  `MemoryStatus` (memory load percentage, total/available physical memory,
  total/available page-file space, total/available virtual address space)
  — closing issue #73 from the parity-loop sweep, the primitive behind a
  `free`-style builtin or general resource reporting. `MEMORYSTATUSEX`'s
  layout independently verified via a compiled mingw-w64 C probe.

## PR #105 — process: add computer_name (GetComputerNameW)
**2026-07-23** · [#105](https://github.com/baileyrd/rusty_win32/pull/105)

- **Added:** `process::computer_name` (`GetComputerNameW`), closing issue
  #72 from the parity-loop sweep — the primitive behind `$HOSTNAME`, a
  shell prompt, or a `hostname` builtin. Adds
  `Win32Error::ERROR_BUFFER_OVERFLOW` (111), needed to distinguish the
  documented "buffer too small, retry with the reported exact size"
  failure from a real error.

## PR #104 — process: add logical_processor_count (GetSystemInfo)
**2026-07-23** · [#104](https://github.com/baileyrd/rusty_win32/pull/104)

- **Added:** `process::logical_processor_count` (`GetSystemInfo`'s
  `dwNumberOfProcessors`), closing issue #71 from the parity-loop sweep —
  the primitive behind an `nproc`-equivalent builtin. No `Result`:
  `GetSystemInfo` has no documented failure mode, matching this crate's
  already-established "never fails" pattern. `SYSTEM_INFO`'s layout
  independently verified via a compiled mingw-w64 C probe, matching this
  crate's usual FFI-struct verification discipline.

## PR #103 — process: add sleep_ms (Sleep)
**2026-07-23** · [#103](https://github.com/baileyrd/rusty_win32/pull/103)

- **Added:** `process::sleep_ms` (`Sleep`), closing issue #70 from the
  parity-loop sweep — the direct primitive behind a `sleep`/`usleep`
  builtin. No `Result`: `Sleep` has no documented failure mode, matching
  this crate's already-established "never fails" pattern (e.g.
  `GetDriveTypeW`).

## PR #102 — volume: add disk_free_space (GetDiskFreeSpaceExW)
**2026-07-23** · [#102](https://github.com/baileyrd/rusty_win32/pull/102)

- **Added:** `volume::disk_free_space` (`GetDiskFreeSpaceExW`), returning a
  `DiskFreeSpace` (`free_bytes_available_to_caller`/`total_bytes`/
  `total_free_bytes`) — closing issue #69 from the parity-loop sweep.
  `volume.rs` already wrapped volume metadata (`GetVolumeInformationW`,
  issue #41) but not free/total space, needed for a `df`-style builtin.

## PR #101 — handle: add handle_information (GetHandleInformation)
**2026-07-23** · [#101](https://github.com/baileyrd/rusty_win32/pull/101)

- **Added:** `handle::handle_information` (`GetHandleInformation`), closing
  issue #68 from the parity-loop sweep. The read-side counterpart to
  `set_inheritable`'s write-only `SetHandleInformation` wrapper — e.g. to
  verify a redirection setup before/after marking a handle inheritable.
  Returns the raw flags bitmask unmodified, matching this crate's existing
  policy-free convention for other raw bitmask fields.

## PR #100 — pipe: add disconnect_server (DisconnectNamedPipe)
**2026-07-23** · [#100](https://github.com/baileyrd/rusty_win32/pull/100)

- **Added:** `pipe::disconnect_server` (`DisconnectNamedPipe`), closing
  issue #67 from the parity-loop sweep. `pipe::create_server`/
  `connect_server` (issue #39) had no way to disconnect and reset a served
  pipe instance for reuse with the next client — a served instance could
  only ever be used once before the whole server had to be recreated.

## PR #99 — console: add pending_input_events (GetNumberOfConsoleInputEvents)
**2026-07-23** · [#99](https://github.com/baileyrd/rusty_win32/pull/99)

- **Added:** `console::pending_input_events`
  (`GetNumberOfConsoleInputEvents`), closing issue #66 from the parity-loop
  sweep — a non-blocking "how many events are queued" check for console
  input, the console-input analog of `handle::pipe_bytes_available`
  (`PeekNamedPipe`) for pipes. `wait_readable` can only answer "is at least
  one event ready" (and blocks for a nonzero timeout); this is an
  instantaneous depth check.

## PR #98 — console: add flush_input (FlushConsoleInputBuffer)
**2026-07-23** · [#98](https://github.com/baileyrd/rusty_win32/pull/98)

- **Added:** `console::flush_input` (`FlushConsoleInputBuffer`), closing
  issue #65 from the parity-loop sweep. Discards every currently-queued,
  not-yet-read input event on a console input handle — dropping stale
  keystrokes buffered during a slow command so they don't replay into the
  next prompt, most noticeable right after `Ctrl-C` interrupts something
  while a user kept typing.

## PR #97 — console: add fill_char/fill_attribute (FillConsoleOutputCharacterW/FillConsoleOutputAttribute)
**2026-07-23** · [#97](https://github.com/baileyrd/rusty_win32/pull/97)

- **Added:** `console::fill_char`/`console::fill_attribute`
  (`FillConsoleOutputCharacterW`/`FillConsoleOutputAttribute`), closing
  issue #64 from the parity-loop sweep. A common line-editor primitive:
  erasing stale characters (and their color/attribute bits) after a
  shorter re-render, e.g. when a redrawn prompt line is shorter than what
  it's replacing — the role a VT `\x1b[K` escape plays for a caller that
  assumes that path, which this crate doesn't assume for every consumer.
  Both return the number of cells actually written, which is less than the
  requested count if the write runs past the end of the screen buffer.

## PR #96 — console: add set_cursor_position (SetConsoleCursorPosition)
**2026-07-23** · [#96](https://github.com/baileyrd/rusty_win32/pull/96)

- **Added:** `console::set_cursor_position` (`SetConsoleCursorPosition`),
  closing issue #63 from the parity-loop sweep. `console.rs` previously only
  *read* cursor position via `window_size`'s underlying
  `GetConsoleScreenBufferInfo` call; a raw-mode line editor (`rusty_lines`)
  doing multi-line prompt redraws needs to reposition the cursor directly,
  with no VT-escape-sequence fallback assumed anywhere else in this crate.
  Reuses the existing private `Coord` struct shape already modeled for
  `window_size`.

## PR #95 — console: add title/set_title (GetConsoleTitleW/SetConsoleTitleW)
**2026-07-23** · [#95](https://github.com/baileyrd/rusty_win32/pull/95)

- **Added:** `console::title`/`console::set_title`
  (`GetConsoleTitleW`/`SetConsoleTitleW`), closing issue #62 from the
  parity-loop sweep — the Windows analog of xterm's OSC title-setting
  escape sequence, a common shell feature (showing cwd/running command in
  the window title) this crate had no primitive for at all before this.
  `title` reports an empty string, not an error, for a console with no
  title set, handling the same "zero return means either buffer-too-small
  or empty" quirk `process::get_env_var` already handles for
  `GetEnvironmentVariableW`.

## PR #94 — job: add is_in_job (IsProcessInJob)
**2026-07-23** · [#94](https://github.com/baileyrd/rusty_win32/pull/94)

- **Added:** `job::is_in_job` (`IsProcessInJob`), closing issue #61 from the
  parity-loop sweep. Checks whether a process already belongs to a given
  job — or, with `job: None`, to *any* job — before calling `assign`.
  Windows automatically nests every child a job member spawns into that
  same job, and some environments (GitHub Actions' Windows runners among
  them, per rush's own `docs/WINDOWS_JOB_CONTROL.md`) start a process
  already job-scoped by an ambient job wrapping the whole step's process
  tree, which would otherwise surface as a surprise `AssignProcessToJobObject`
  failure.

## PR #93 — process: add image_path (QueryFullProcessImageNameW)
**2026-07-23** · [#93](https://github.com/baileyrd/rusty_win32/pull/93)

- **Added:** `process::image_path` (`QueryFullProcessImageNameW`), closing
  issue #60 from the parity-loop sweep. Completes `list_processes`'s
  `ProcessEntry::exe_file` (issue #21, `PROCESSENTRY32W.szExeFile`), which
  is only ever a bare filename, not a full path — needed for a `ps`-style
  listing that wants to show each process's real executable path. Unlike
  this crate's other growing-buffer calls, `QueryFullProcessImageNameW`
  doesn't report the size actually required on a "buffer too small"
  failure, so this doubles the buffer and retries instead of growing to an
  exact reported size.

## PR #92 — process: add process_id_of (GetProcessId)
**2026-07-23** · [#92](https://github.com/baileyrd/rusty_win32/pull/92)

- **Added:** `process::process_id_of` (`GetProcessId`), the reverse of
  `open_by_pid`'s (issue #13) pid-to-`HANDLE` mapping — closing issue #59
  from the parity-loop sweep. Needed anywhere rush holds a `HANDLE` (e.g.
  `spawn_suspended`'s own `process` handle) and needs to report/print its
  numeric pid without having cached it separately.

## PR #91 — path: add full_path (GetFullPathNameW)
**2026-07-23** · [#91](https://github.com/baileyrd/rusty_win32/pull/91)

- **Added:** `path::full_path` (`GetFullPathNameW`), resolving a relative
  path (or one with `.`/`..` components) to its fully qualified absolute
  form — closing issue #58 from the parity-loop sweep. Follows
  `search_path`/`short_path`/`long_path`'s existing two-attempt
  growth-buffer pattern. Unlike `short_path`/`long_path`, purely lexical —
  `GetFullPathNameW` never touches the filesystem, so it succeeds even for
  a path that doesn't exist.

## PR #90 — handle: add get_std_handle/set_std_handle (GetStdHandle/SetStdHandle)
**2026-07-23** · [#90](https://github.com/baileyrd/rusty_win32/pull/90)

- **Added:** `handle::get_std_handle`/`handle::set_std_handle`
  (`GetStdHandle`/`SetStdHandle`) plus the `STD_INPUT_HANDLE`/
  `STD_OUTPUT_HANDLE`/`STD_ERROR_HANDLE` slot constants, closing issue #57
  from the parity-loop sweep. `process.rs`'s own `spawn_suspended` doc
  comment already described redirection as "swapping the parent's
  std-handle slots before spawning, matching `winstdio`'s existing model in
  rush" — this crate previously assumed that primitive without owning it.
  `get_std_handle` returns `Ok(None)` (not `Err`) for `GetStdHandle`'s
  documented "no handle assigned" `NULL` outcome, distinct from an actual
  call failure (`INVALID_HANDLE_VALUE`).

## PR #89 — process: add get_env_var/set_env_var (GetEnvironmentVariableW/SetEnvironmentVariableW)
**2026-07-23** · [#89](https://github.com/baileyrd/rusty_win32/pull/89)

- **Added:** `process::get_env_var`/`process::set_env_var` — live single-
  variable environment access, closing issue #56 from the parity-loop sweep.
  Complements `environment_snapshot`'s full-block read (issue #19): `export
  NAME=value`/`unset NAME`/reading one `$VAR` need per-variable get/set, not
  just a startup-time snapshot. `get_env_var` reports `Ok(None)` for an unset
  variable (matching `path::search_path`'s "not found isn't an error"
  convention) and handles `GetEnvironmentVariableW`'s documented quirk where
  a set-but-empty variable also returns 0, distinguished from "not found"
  only by `GetLastError` reporting success. `set_env_var`'s `value: None`
  deletes the variable, per `SetEnvironmentVariableW`'s own contract.

## PR #88 — fs: add read_dir (FindFirstFileW/FindNextFileW/FindClose)
**2026-07-23** · [#88](https://github.com/baileyrd/rusty_win32/pull/88)

- **Added:** `fs::read_dir`, returning a `ReadDir` iterator of `DirEntry`
  (name, attributes, size, and the three `FILETIME` timestamps) — the Win32
  primitive behind directory listing, closing issue #55 from the parity-loop
  sweep. Follows the same "opening call already returned the first item"
  shape as `process::list_processes`'s `Process32FirstW` loop; the search
  handle closes via `FindClose` on `Drop`. Matches Unix `readdir` in
  reporting `.`/`..` as real entries rather than filtering them.

## PR #87 — path: add current_dir/set_current_dir (GetCurrentDirectoryW/SetCurrentDirectoryW)
**2026-07-23** · [#87](https://github.com/baileyrd/rusty_win32/pull/87)

- **Added:** `path::current_dir`/`path::set_current_dir` — the actual Win32
  primitives behind `cd`/`pwd`. Closes issue #54, the first item worked from
  a new parity-loop pass against the real Win32 API surface
  (`gap-analysis.md`, PR #86) — a systematic function-level sweep (mingw-w64
  headers as a local proxy for `windows-sys`) rather than the round-2
  assessment's needs-driven inventory. Surprising finding from that sweep:
  nothing in this crate wrapped either primitive at all before this.

## PR #86 — Add gap-analysis.md: parity-loop assessment vs. the real Win32 API surface
**2026-07-23** · [#86](https://github.com/baileyrd/rusty_win32/pull/86)

- **Added:** `gap-analysis.md`, a function-level Win32 API coverage sweep.
  32 candidate gaps identified (21 with a concrete rush/rusty_lines
  consumer, 11 plausible-but-not-yet-built coreutils-style builtins), filed
  as issues #54–#85. ~25 additional candidates found with no clear
  consumer, listed but not filed. Registry/ACLs/services/networking/ConPTY
  reconfirmed out of scope.

## PR #53 — job: narrow process_ids to Vec<u32>, matching every other pid in this crate
**2026-07-23** · [#53](https://github.com/baileyrd/rusty_win32/pull/53)

- **Changed:** `job::process_ids` now returns `Vec<u32>` instead of
  `Vec<usize>` — closes the round-2 assessment's API-consistency wart.
  Every other pid-carrying value in the public surface (`ProcessEntry.pid`,
  `JobMessage.pid`, `SpawnedProcess.process_id`, `open_by_pid`'s parameter)
  was already `u32`; `process_ids` alone exposed the raw
  `JOBOBJECT_BASIC_PROCESS_ID_LIST` wire format's pointer-sized
  (`ULONG_PTR`) width, which exists for struct alignment, not because a
  pid is ever wider than 32 bits. Breaking, pre-1.0.

## PR #52 — Add watch module: filesystem change notification (ReadDirectoryChangesW)
**2026-07-23** · [#52](https://github.com/baileyrd/rusty_win32/pull/52)

- **Added:** `watch::open_directory`/`watch::read_changes`, wrapping
  `ReadDirectoryChangesW` — closes the round-2 assessment's final item, and
  the only one that genuinely required `OVERLAPPED` I/O. No current `rush`
  feature (no file-watch builtin) asks for this; added as a standard
  building block a maturing shell eventually wants.
- `ReadDirectoryChangesW` has no way to bound how long it blocks other
  than overlapped completion — `read_changes` wraps that behind the same
  `Option<u32>` timeout convention `process::wait` already uses, cancelling
  the pending read via `CancelIoEx` on timeout so a caller never risks an
  unbounded hang.
- A buffer overflow (more change records in one burst than the internal
  64 KiB buffer holds) reports `ERROR_NOTIFY_ENUM_DIR` rather than a
  silently truncated result — Windows' own signal that changes were
  missed. `error.rs` gained this named constant.

## PR #51 — path: add short_path/long_path (GetShortPathNameW/GetLongPathNameW)
**2026-07-23** · [#51](https://github.com/baileyrd/rusty_win32/pull/51)

- **Added:** `path::short_path`/`path::long_path`, normalizing between a
  legacy 8.3 short name (e.g. `PROGRA~1`) and its long form — closes the
  round-2 assessment's last speculative item. A rare but real source of
  path-comparison surprises this crate's reparse-point-aware `fs::final_path`
  doesn't otherwise cover; no known consumer today.
- Both reuse `search_path`'s existing two-attempt buffer-growth pattern.

## PR #50 — Add volume module: drive/volume enumeration (GetLogicalDrives/GetDriveTypeW/GetVolumeInformationW)
**2026-07-23** · [#50](https://github.com/baileyrd/rusty_win32/pull/50)

- **Added:** `volume::logical_drives`/`drive_type`/`volume_information`,
  closing the round-2 assessment's remaining speculative item — a
  distinctly Windows-shaped gap (multi-root filesystem model, no Unix
  analog at all) rather than a fix for any current `rush`/`rusty_lines`
  need.
- `drive_type` never fails (matches `GetDriveTypeW`'s own contract — no
  `GetLastError` failure mode exists for it).
- `VolumeInformation`'s `file_system_flags` is exposed as a raw bitmask,
  matching this crate's existing policy-free convention for other raw
  bitmask fields (`fs::FILE_ATTRIBUTE_*`, `console::ENABLE_*`).

## PR #49 — console: add write_key_events for non-character virtual-key codes
**2026-07-23** · [#49](https://github.com/baileyrd/rusty_win32/pull/49)

- **Added:** `console::write_key_events`, extending `write_char_events`'s
  test-input-synthesis technique to non-character keys (arrows, Home/End,
  function keys, …) that carry no `uChar` at all — closes the round-2
  assessment's last item. Blocked a Windows-side test suite for
  `rusty_lines`' history/cursor/keymap navigation until now.
- Adds the `VK_*` virtual-key-code constants and `ENHANCED_KEY` (auto-set
  for the navigation-cluster keys, matching what a real keyboard driver
  always sets for them).
- Looks up a real hardware scan code via `MapVirtualKeyW` rather than
  leaving it `0` — this crate's first non-`kernel32` link (`user32.dll`),
  an expansion the README's own module docs already anticipated (alongside
  `advapi32.dll`).
- New test empirically proves the left-arrow key round-trips through
  `ENABLE_VIRTUAL_TERMINAL_INPUT` translation as the standard VT100
  cursor-left escape sequence (`\x1b[D`).

## PR #48 — Add pipe module: named pipes (CreateNamedPipeW/ConnectNamedPipeW/WaitNamedPipeW)
**2026-07-23** · [#48](https://github.com/baileyrd/rusty_win32/pull/48)

- **Added:** `pipe::create_server`/`connect_server`/`wait_for_server`/
  `open_client`, wrapping `CreateNamedPipeW`/`ConnectNamedPipeW`/
  `WaitNamedPipeW`/`CreateFileW` — closes the round-2 assessment's named-pipe
  gap. `handle::create_pipe`'s anonymous pipes have no name an arbitrary
  already-running program can open; rush's own `docs/WINDOWS_JOB_CONTROL.md`
  and `docs/CAPABILITY_GAPS.md` both name this as the missing primitive
  blocking process substitution (`<(cmd)`) and `coproc` on Windows.
- `connect_server` treats the documented `ERROR_PIPE_CONNECTED` race
  (client connects before the server calls `ConnectNamedPipeW`) as success,
  not a failure — the same pattern `process::list_processes` already uses
  for `ERROR_NO_MORE_FILES`.
- No `OVERLAPPED` support yet, matching this crate's existing synchronous-
  I/O convention elsewhere (`handle`'s anonymous pipes, `console::read`).
- `error.rs` gained `ERROR_PIPE_CONNECTED`/`ERROR_PIPE_BUSY` named
  constants.

## PR #47 — job: add resource-limit set/query and CPU/IO accounting
**2026-07-23** · [#47](https://github.com/baileyrd/rusty_win32/pull/47)

- **Added:** `job::set_resource_limits`/`job::limits` (memory, per-process
  and per-job CPU-time, and active-process-count limits) and
  `job::accounting` (`JobObjectBasicAndIoAccountingInformation`) — closes
  the round-2 assessment's Job-Object item. `rush`'s `ulimit` is flat
  "not supported" on Windows today; Job-Object limits are that crate's own
  documented answer for the only realistic partial fix, and the struct
  fields these use were already modeled bit-for-bit for
  `set_kill_on_close`, just never set beyond its one `LimitFlags` bit.
- `job::accounting` is Windows' real analog of POSIX `cutime`/`cstime`: CPU
  time aggregated across every process a job has *ever* contained,
  including already-exited ones — unlike `process::times`, which only
  reports one still-open process handle's own times.
- **Note:** `set_resource_limits` replaces the job's entire limit-info
  block in one call, same as `set_kill_on_close`/`clear_kill_on_close` —
  documented as a caveat rather than solved, since combining both concerns
  in one `SetInformationJobObject` call is a separate primitive this PR
  doesn't add.
- `Timespec` (`time.rs`) gained a `Default` impl, needed for
  `JobAccounting`'s derive.

## PR #46 — process: add GetProcessTimes wrapper (process::times)
**2026-07-23** · [#46](https://github.com/baileyrd/rusty_win32/pull/46)

- **Added:** `process::times`, wrapping `GetProcessTimes` — closes the
  round-2 assessment's other must-have. Without this, rush's `time` builtin
  had no way to report real per-child CPU time on Windows and always
  printed a hardcoded zero — a visibly wrong output, not merely a missing
  feature.
- `kernel_time`/`user_time` are elapsed *durations* since process creation,
  not wall-clock timestamps — reuses `Timespec`'s shape the same way
  `time::now_monotonic`'s result already does for a non-wall-clock value.
- `PROCESS_QUERY_LIMITED_INFORMATION` added as the narrowest `OpenProcess`
  access right this call actually needs.

## PR #45 — process/console: new-process-group spawn + targeted Ctrl-Break delivery
**2026-07-23** · [#45](https://github.com/baileyrd/rusty_win32/pull/45)

- **Added:** `process::spawn_suspended`'s `new_process_group` parameter
  (`CREATE_NEW_PROCESS_GROUP`) and `console::generate_ctrl_event`
  (`GenerateConsoleCtrlEvent`) — closes the round-2 capability assessment's
  top-ranked gap: nothing previously let a caller interrupt one child
  without hitting every process attached to the console at once.
- `CTRL_C_EVENT` can only ever be broadcast console-wide by Windows' own
  design (documented and tested: a nonzero process-group id fails with
  `ERROR_INVALID_PARAMETER`); targeting one child's group needs
  `CTRL_BREAK_EVENT` instead.
- **Changed:** `spawn_suspended`'s signature (new `new_process_group: bool`
  parameter) — a breaking change, acceptable pre-1.0.
- Note: several PRs (#22–#35) shipped between this entry and PR #9 below
  without a `RELEASE_NOTES.md` entry each — a backlog gap, not something
  this entry backfills; see `docs/CAPABILITY_ASSESSMENT.md` for that work's
  own record instead.

## PR #9 — process: add wait_any, a WaitForMultipleObjects(bWaitAll=FALSE) wrapper
**2026-07-18** · [#9](https://github.com/baileyrd/rusty_win32/pull/9)

- **Added:** `process::wait_any`, blocking on whichever of a slice of process
  handles exits first — the multi-handle counterpart to `process::wait`,
  needed for rush's `wait -n` without looping a zero-timeout `wait` over every
  tracked handle and sleeping between sweeps.
- Bounded by `WaitForMultipleObjects`'s own `MAXIMUM_WAIT_OBJECTS` (64) limit;
  exceeding it reports `ERROR_INVALID_PARAMETER` (the real call's own error),
  not a crate-invented one.

## PR #8 — job: add clear_kill_on_close, the reverse of set_kill_on_close
**2026-07-18** · [#8](https://github.com/baileyrd/rusty_win32/pull/8)

- **Added:** `job::clear_kill_on_close`, letting a job's member processes
  survive every handle to the job closing — including implicitly at the
  shell's own exit. Backs the `disown` builtin: without this, a caller
  couldn't just stop tracking a job and drop its handle, since kill-on-close
  would still fire.

## PR #7 — process: let spawn_suspended override the child's environment block
**2026-07-18** · [#7](https://github.com/baileyrd/rusty_win32/pull/7)

- **Added:** `process::environment_block` plus an `environment` parameter on
  `spawn_suspended` to hand a `CREATE_SUSPENDED` child an explicit,
  from-scratch environment block instead of inheriting the parent's real OS
  environment. Needed because rush's `vars` module never calls
  `std::env::set_var`/`remove_var` — it keeps its own exported-variable table,
  which can otherwise silently diverge from what a spawned child would
  inherit by default.

## PR #6 — Add console::write_char_events (WriteConsoleInputW) for test-driven input synthesis
**2026-07-18** · [#6](https://github.com/baileyrd/rusty_win32/pull/6)

- **Added:** `console::write_char_events`, synthesizing real console key
  events via `WriteConsoleInputW` — the standard technique console
  automation tools use to inject keystrokes.
- Not a rush/`rusty_lines` production need on its own: this exists so a test
  can drive a raw-mode reader through its real Windows I/O path end to end
  (the Windows analog of writing into one end of a Unix pty), without
  needing ConPTY.
- Its own test empirically proves the `WriteConsoleInputW` →
  `ENABLE_VIRTUAL_TERMINAL_INPUT` → `ReadFile` round trip reproduces the same
  bytes a real keypress would.

## PR #5 — Phase 4: raw-mode console primitives (GetConsoleMode/SetConsoleMode)
**2026-07-17** · [#5](https://github.com/baileyrd/rusty_win32/pull/5)

- **Added:** `console::get_mode`/`set_mode` (`GetConsoleMode`/`SetConsoleMode`,
  the Windows analog of `tcgetattr`/`tcsetattr`), `console::read` (raw
  `ReadFile`), `console::wait_readable` (`WaitForSingleObject`, the analog of
  `poll` on a single console handle), and `console::window_size`
  (`GetConsoleScreenBufferInfo`, the analog of `TIOCGWINSZ`).
- **Fixed:** switched the test suite's console-handle acquisition from
  `GetStdHandle` to `CreateFileW("CONIN$"/"CONOUT$", ...)` after
  `GetStdHandle` kept returning a stale redirected handle on `windows-latest`
  CI even after `AllocConsole` attached a real console.
- Corrected an assumption carried over from rush's own backend-analysis doc:
  Windows Ctrl-C-at-idle-prompt does **not** already work via `rusty_lines`'s
  non-Unix path — that path has no Ctrl-C handling at all without this.
- Deliberately not ConPTY: `CreatePseudoConsole` hosts a *child* process's
  console session (what a terminal emulator does), not a process reading its
  own inherited stdin the way `rusty_lines` does — `GetConsoleMode`/
  `SetConsoleMode` is the actual analog of `tcgetattr`/`tcsetattr` here.

## PR #4 — Phase 5: time module (QueryPerformanceCounter/GetSystemTimePreciseAsFileTime)
**2026-07-17** · [#4](https://github.com/baileyrd/rusty_win32/pull/4)

- **Added:** `time::now_monotonic`/`time::now_realtime` — the Windows analog
  of `rusty_libc::vdso`'s "read kernel-shared memory instead of syscalling"
  fast path (`QueryPerformanceCounter` is documented to be backed by the same
  `KUSER_SHARED_DATA` page).
- Lowest-priority module per rush's own backend analysis: no `cfg(not(unix))`
  call site in rush needs it today (rush uses `std::time` exclusively, and
  std's own Windows backend already uses `QueryPerformanceCounter`
  internally) — added for `rusty_lines`/completeness, not an open rush gap.

## PR #3 — Phase 3: process + job modules (spawn_suspended, Job Objects)
**2026-07-17** · [#3](https://github.com/baileyrd/rusty_win32/pull/3)

- **Added:** raw `CreateProcessW`-based `process::spawn_suspended`/`resume`/
  `wait`, plus the full `job` module (`create`/`assign`/`set_kill_on_close`/
  `terminate`/`process_ids`) — the primitives rush's Windows background-job
  design (`&`, `jobs`, `wait`, `kill`, `$!`) is built against. Narrowly
  scoped to what job-object-integrated spawning needs, not a replacement for
  `std::process::Command` (ordinary foreground spawn/wait already works via
  `std::process::Command`, which resolves to the same underlying calls).

## PR #2 — Phase 2: handle module (DuplicateHandle/CreatePipe/SetHandleInformation/CloseHandle)
**2026-07-17** · [#2](https://github.com/baileyrd/rusty_win32/pull/2)

- **Added:** `handle::create_pipe`/`duplicate`/`set_inheritable`/`close` — the
  Windows counterpart of Unix `dup`/`pipe2`/`close`, closing rush's
  fd-3-and-up gap at the raw-primitive level. The integer-to-`HANDLE` map
  that gives fd 3+ and `{name}>` varfd redirects any meaning stays a
  follow-up in rush itself, deliberately not this crate.

## PR #1 — Bootstrap rusty_win32: Phase 1 (Win32Error, console ctrl handler)
**2026-07-17** · [#1](https://github.com/baileyrd/rusty_win32/pull/1)

- **Added:** `error::Win32Error` (a `GetLastError()` wrapper with named
  `ERROR_*` constants, `Display`, `core::error::Error`, and an opt-in `std`
  feature adding `From<Win32Error> for std::io::Error`) and
  `console::install_ctrl_handler`/`remove_ctrl_handler`
  (`SetConsoleCtrlHandler`) — closing rush's single highest-value,
  lowest-risk Windows gap: `trap 'cmd' TERM` was silently accepted on Windows
  but had nothing installed to ever fire it.
- Established the crate's shape: `#![no_std]`-where-possible, `extern
  "system"` FFI against `kernel32.dll`, safe wrappers returning
  `Result<T, Win32Error>` with `unsafe` confined to FFI declarations and
  raw-handle-taking functions.
