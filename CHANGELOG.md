# Changelog

Format loosely follows [Keep a Changelog](https://keepachangelog.com/);
version-bump rule is [`docs/versioning.md`](docs/versioning.md) §2 (at
`0.y.z`, any public-API change — additive or breaking — bumps `y`;
`z` is reserved for changes that touch no public item's shape).

This changelog starts with the adoption of that policy. Everything
before it (Fs, Process, Events, Track P, the error model, the parity
regime) landed under no formal version-bump discipline at all — it's
summarized once, below, rather than reconstructed bump-by-bump after
the fact, since nothing external ever pinned to a specific version
during that period to make the reconstruction meaningful.

Three independently-versioned lines, per `docs/versioning.md` §1:
**the PAL group** (`platform`/`platform-linux`/`platform-windows`/
`platform-mock`/`platform-bsd`, plus the dev-only `platform-parity`,
sharing one number), **`winargv`**,
and **`coreutils`**.

## PAL group (`platform` / `platform-linux` / `platform-windows` / `platform-mock` / `platform-bsd` / `platform-parity`)

### 0.25.0

- **Added `platform-windows`'s `track-w` feature — `rusty_win32` adopted
  as a real dependency (D-15).** The Windows counterpart of D-12: an
  off-by-default feature routing curated call families through
  `rusty_win32`'s hand-written `extern "system"` declarations instead of
  windows-sys, as a rev-pinned git dependency (one source of truth, no
  vendored fork), migrated call-by-call. First family:
  `sys::fileio::read`/`write`, deliberately mirroring Track P's own first
  slice so the two adoptions stay comparable. Both configurations produce
  bit-identical `PlatformError`s — same `kind_of_win32` table, same
  `OsCode::Win32` — so the whole platform-windows suite re-runs under
  `--features track-w` as the equivalence test rather than needing a
  parallel one; that run is a new windows+stable CI leg, with a
  cross-compile clippy pre-check beside it.

  rusty_win32 had been a *donor* since the extraction map opened (D2, D5,
  D9's console cluster). This does not retract any of that porting —
  what's extracted stays extracted — it adds a second, narrower
  relationship alongside it for the calls where the donor already
  declares the import correctly.

  **Not a lower tier, and recorded as such rather than left to the
  symmetric feature name to imply:** Track P descends below libc to the
  kernel ABI, whereas Windows publishes no supported tier beneath a
  documented DLL export — both configurations reach the same
  `kernel32!ReadFile`. `track-w` swaps the binding's provenance
  (hand-written and reviewed, no `windows-targets` import-lib machinery,
  `no_std`-capable), not its depth, so **D-1 stands unchanged** and
  windows-sys remains the default floor. Families rusty_win32 has no
  binding for at the pinned rev stay on windows-sys in both
  configurations — `sys::nt` above all, whose `NtCreateFile`
  handle-relative opens are this backend's entire capability model —
  the same way `fchmodat` stays on libc in both Track P configurations.
  Full write-up: `docs/learning/003-…`.

  Two things worth knowing for the next rev bump. First, MSRV: like
  rusty_libc, rusty_win32's floor (1.88) sits above this workspace's
  (1.75), which is exactly why the feature is opt-in — the MSRV CI leg
  never resolves it on. Second, and new: adopting it required a manifest
  fix *upstream* (rusty_win32 edition 2024 → 2021, plus a declared
  `rust-version = "1.88"` and its own MSRV CI job). Cargo parses the
  manifest of every resolved dependency, including an optional one whose
  feature is off, so the donor's `edition` was failing this workspace's
  1.75 leg on the dependency's mere presence — before compiling a line.
  A dependency's edition is a constraint on every consumer that merely
  *lists* it, in a way its `rust-version` is not.

  `y`, not `z`: a new Cargo feature is an addition to the crate's public
  interface — nameable by any consumer, and resolvable — even though no
  `pub` item's shape moved. §2's "stop treating additive changes as
  free" applies.

### 0.24.0

- **Added `platform::fs::AnonymousFile::create_memfd`** — `memfd_create`,
  an anonymous, memory-backed `File` with no filesystem namespace entry
  at all. D11's own material (cited since the extraction map opened as
  the load-bearing invariant for a raw `clone(SIGCHLD)` fork to be
  sound) but never surfaced as an API — landed independently while
  digging into the still-parked, still-undecided fork/execve vs
  `posix_spawn` question (`docs/decision-request-fork-execve.md`'s
  option 3), not as part of resolving it. Deliberately a new, separate
  trait rather than a `Dir` method: `memfd_create` takes no path and
  touches no directory capability, so forcing it onto `Dir` would mean
  an always-unused `&self` receiver on every call. Linux: plain
  `libc::memfd_create` + `MFD_CLOEXEC`, no `track-p` gate needed — the
  libc wrapper predates this workspace's MSRV floor. Windows:
  `Unsupported`, no numbered divergence (a missing-donor-shape gap,
  `WindowsTun`'s own precedent — `CreateFileMappingW`'s anonymous
  mapping is a fixed-size shared-memory mapping, not a growable
  byte-stream `File`). `platform-mock`'s own `MockAnonymousFile` gives
  a real, non-`Unsupported` implementation (an in-memory buffer, the
  same shape `MockFile` already has) since nothing about this
  capability needs faking the way `Csprng`/`Sandbox` do. See
  `docs/behavior/fs.md` for the full contract.

  `y`, not `z`: a new public trait on `platform`, new public structs
  on each backend.

### 0.23.0

- **Added `platform::term::{ConsoleState, ConsoleAcquisition}`** — the
  console-*acquisition* facet extraction map D9 flagged but left unbuilt
  (`AttachConsole`/`AllocConsole`/`FreeConsole`, for a GUI-subsystem
  Windows process that starts with no console at all). Built
  speculatively, without a confirmed live consumer, on the owner's
  explicit call — the same posture PTY hosting (D13) was built under.
  `ConsoleAcquisition` is a deliberately separate opt-in trait from
  `Terminal` (only `WindowsTerminal` implements it), mirroring
  `JobControlTerminal`'s own asymmetry in the other direction. Landed
  with `docs/design-discussion-console.md` (the reconciliation pass,
  since the `rusty_naner` donor D9 actually attributes this facet to
  wasn't in reach — built instead from the *other* D9 donor,
  `rusty_win32`'s real `AllocConsole`/`AttachConsole`/`FreeConsole`
  primitives, plus its own `CONIN$`/`CONOUT$`-reopen technique promoted
  from test-only to production code) and a new registered divergence
  (`docs/divergences.md` #012: no Linux implementor at all, not a
  runtime `Unsupported`, since the GUI-subsystem/console-subsystem split
  this facet exists for has no Unix analog).

  `y`, not `z`: a new public trait and enum on `platform`.

### 0.22.1

- **Extracted the shared parity assertions into `platform-parity`**, a
  new test-support crate. `assert_net_behavior`/`assert_unix_behavior`/
  `assert_udp_behavior` and `assert_csprng_behavior`/
  `assert_credential_store_behavior`/`assert_trust_anchors_behavior` now
  live once; each backend's `tests/*_parity.rs` records only *which* sets
  apply to it, plus its own OS-specific expectations. Mock conformance
  moved to `platform-mock/tests/parity_conformance.rs`, where it runs
  once rather than being re-declared in every backend suite.

  `z`, not `y`: no public item's shape changed. `platform-parity` is a
  `dev-dependency` only and never enters a shipped dependency graph.

  This is the follow-up `net_parity.rs` and `security_parity.rs` each
  recorded in their own doc comments — *extract once a third backend
  would otherwise mean a third copy*. `platform-bsd` made net's third
  (rustils#48/#86) and `TrustAnchors` made security's (rustils#88).

  **The trigger was set at the right place, and the copies had already
  started to rot by the time it fired.** Two ways, both found while
  extracting:
  - `platform-bsd`'s `assert_net_behavior` had lost two explanatory
    comments the other two still carried — harmless in itself, and
    exactly how three copies stop being one spec.
  - `assert_credential_store_behavior` had genuinely **diverged**:
    Windows scoped its test service name by process id, Linux used a
    fixed string. Against a real per-user OS credential store, two
    concurrently-running test binaries sharing a fixed name can see each
    other's writes. The pid-scoped version is correct and is what the
    shared crate carries, so extracting fixed a latent flake rather than
    only deduplicating text — the same class of bug the net suite had
    already fixed once, when `assert_unix_behavior`'s socket path gained
    a per-backend label.

  Coverage is unchanged: 175 unique tests before and after. The only
  renames are `mock_security_conforms`/`linux_security_conforms` →
  `mock_csprng_conforms`/`linux_csprng_conforms`, which is what those
  assertions actually test. The drop in total *executions* (179 → 176)
  is the mock suite running once instead of three times.

  `Sandbox` deliberately has no shared set: its whole contract is a
  `SandboxStatus` that legitimately differs per backend and per host
  kernel, so there is no cross-backend behavior to assert.

  Not extracted, and still two copies: the Fs `parity.rs` suites
  (linux/windows). The rule is to extract at the third, and Fs has no
  third backend.

### 0.22.0

- **Added `platform::security::TrustAnchors`** (rustils#88) — the fourth
  `security` slice, beside `Csprng`/`CredentialStore`/`Sandbox`. One
  method, `load_anchors() -> Result<Vec<Vec<u8>>>`: the OS's root
  certificates as raw DER. Implemented on all four backends
  (`platform-linux`, `platform-bsd`, `platform-windows`,
  `platform-mock`). New public trait, hence the `y` bump.

  This is the "B1" slice `docs/design-discussion-tls.md` researched and
  parked, gated in when `rusty_tls` decided to drop
  `rustls-native-certs` and hand-roll anchor loading (rustils#70,
  rusty_tls#24). It is deliberately the *only* TLS-adjacent thing that
  will ever land here: it answers "where does this OS keep its roots and
  how do I read the bytes" — OS personality — and never parses,
  validates, or verifies anything. No chain building, no signature
  checks, no ASN.1. Raw DER at the boundary, the §5.2 byte-oriented
  instinct applied to certificates.

  Contract, identical on every backend: per-anchor errors are tolerated
  and skipped (real stores carry damaged entries; one must not cost the
  caller the other several hundred); **zero usable anchors is
  `ErrorKind::NotFound`, never `Ok(vec![])`** (an empty set would trust
  nothing and fail every connection with a confusing per-connection
  error); and every call re-reads, so a machine whose store just changed
  doesn't keep serving the old set.

  Mechanisms: Linux probes `SSL_CERT_FILE`, then `SSL_CERT_DIR`, then
  the first existing distro bundle file, then the first existing distro
  certificate directory — first match wins *exclusively*, never a union.
  Bundle-before-directory is what lets one policy cover every distro
  without special-casing: RHEL ships only a bundle, Debian ships both a
  bundle and a directory of hashed symlinks over the same certificates,
  so preferring the bundle reads one file instead of several hundred
  symlinks and never double-loads. Windows enumerates the ROOT store
  (`CertOpenSystemStoreW` + `CertEnumCertificatesInStore`). macOS uses
  `SecTrustCopyAnchorCertificates`; the other BSDs use the same file
  probing as Linux with BSD paths.

  **Documented as best-effort, with three fidelity limits no
  implementation can fix** — the same ones `rustls-native-certs` carries
  industry-wide, and the same three `rusty_tls`'s own docs reproduce
  independently: Windows' ROOT store is lazily populated so enumeration
  can miss a root the chain engine would have fetched; macOS's one-call
  anchor API returns built-in roots without per-domain trust settings;
  and a flat DER list cannot express distrust at all, so a consumer can
  accept a chain the OS itself would reject. The honest alternative —
  the OS's own chain *verification* API — stays unoffered because Linux
  has no counterpart, and a Linux backend would mean hand-rolling X.509
  path validation: the cryptography this workspace refuses, through a
  door labelled "narrow."

- `platform-bsd` gained its first non-`net` surface (rustils#88). It also
  gained its first non-`libc` FFI: `ffi::security_framework`, a curated
  set of Security.framework/CoreFoundation externs, hand-declared
  because `libc` doesn't cover Apple's frameworks and this workspace
  takes no binding crate for them. Its doc comment records Core
  Foundation's Create/Copy-vs-Get ownership rules, which every `unsafe`
  block in the Darwin path cites by name.

- `platform-bsd/tests/security_parity.rs` is new; the Linux and Windows
  security parity suites grew a `TrustAnchors` section. All three copies
  of `assert_trust_anchors_behavior` are textually identical, and this
  is the third copy — the recorded follow-up to extract the shared
  assertions into one crate is now due, on the same terms the net parity
  suites already record.

### 0.21.0

- **Renamed `platform-macos` → `platform-bsd`, widening its `cfg` gate
  from `target_os = "macos"` to
  `any(macos, freebsd, openbsd, netbsd, dragonfly)`** (rustils#86,
  follow-up to #48). Every public item renames with it:
  `MacosNet`/`MacosTcpStream`/`MacosTcpListener`/`MacosUnixStream`/
  `MacosUnixListener`/`MacosUdpSocket` → `Bsd*`. Breaking for any
  consumer naming the old crate or types — hence the `y` bump per
  `docs/versioning.md` §2 — though nothing in this workspace or
  outside it imports them yet, which is precisely why the rename
  happened now rather than after `rusty_tokio` picked the crate up.

  Forced the same way #48 itself was: `rusty_tokio`#116 wanted the
  `kevent` reactor on FreeBSD/OpenBSD, found no `platform` socket layer
  to sit it on for anything but Darwin, and was about to hand-roll a
  *third* socket lifecycle against raw `libc` — the exact duplication
  #48 was filed to stop, one OS over.

  **No implementation change.** This is the part worth being precise
  about, because the issue asked for it to be verified rather than
  assumed. Of the three BSD-vs-Linux differences #48 documented, only
  the third (`sin_len`/`sin6_len`/`sun_len`, the 4.4BSD sockaddr
  layout) is universal across the BSDs. The other two are Darwin's
  alone: FreeBSD, OpenBSD, NetBSD and DragonFly all *do* have
  `SOCK_CLOEXEC`/`SOCK_NONBLOCK` and `accept4`. The crate's existing
  Darwin-shaped code is therefore the *intersection* of the five
  targets — correct everywhere, optimal on one — and widening the gate
  needed no new code paths. The cost is a fork+exec race between
  `socket`/`accept` and the follow-up `fcntl(F_SETFD, FD_CLOEXEC)`
  which four of the five targets could have avoided atomically;
  accepted deliberately (one code path over two) and documented at
  `sys::net::set_cloexec`, `ffi::libc_surface`, and the crate root,
  along with what closing it would take. `ffi::libc_surface` stays the
  intersection on purpose: admitting `SOCK_CLOEXEC`/`accept4` would
  compile on four targets and break the fifth.

- **Real-BSD CI** (rustils#86). Two new jobs run the full
  `platform`/`platform-mock`/`platform-bsd` suite inside a real
  FreeBSD and a real OpenBSD VM (`vmactions/*-vm`), joining the
  `macos-latest` job #48/#53 added. `cross-compile-check` also grew
  `x86_64-apple-darwin`/`x86_64-unknown-freebsd`/`x86_64-unknown-netbsd`
  clippy legs as a fast pre-check.

  This was the issue's own condition — #53's macOS runner caught a
  genuine Darwin `AF_UNIX` divergence that `cargo check --target`
  could not have, so widening a gate on inference alone was explicitly
  ruled out. Stated plainly, since a green CI now means less than it
  looks: **three of the five targets in the gate are executed**
  (macOS, FreeBSD, OpenBSD), **NetBSD is compiled but never run**, and
  **DragonFly is neither** — tier 3, no prebuilt `std`, no runner. The
  last two are in the gate by inheritance from FreeBSD's socket
  surface, which is an inference, not a measurement.

- **Fixed a real OpenBSD `AF_UNIX` divergence the new VM job caught on
  its first run** — the second time a real-OS leg has immediately
  justified itself here, after #48's Darwin bug. `getsockname` on a
  *bound* socket returns the path followed by the rest of `sun_path` as
  NUL padding rather than shrinking `len` to the path length, so
  `local_addr()` reported `Some("/tmp/….sock\0\0…")` instead of
  `Some("/tmp/….sock")`. `from_sockaddr_un` had popped a single
  trailing NUL, which is exactly right on Linux and Darwin (where `len`
  is `offset + strlen + 1`) and wrong wherever `len` spans the buffer.

  Now truncates at the first NUL inside the `len`-bounded window —
  `sun_path` read as the C string it is. Correct whether or not a
  kernel shrinks `len`, bound or unbound, and it subsumes the Darwin
  fix from #48 (an all-zero buffer truncates to empty, i.e. `None`).
  Sound because `to_sockaddr_un` refuses embedded NULs, so no real path
  can be cut short. No `docs/divergences.md` entry: this brings OpenBSD
  in line with the documented `platform::net` contract rather than
  recording a permanent difference from it — same call #48 made.

  Worth stating plainly: every static check passed on the buggy code.
  `cargo check`/`clippy` for freebsd and netbsd, the macOS leg, and the
  FreeBSD VM leg were all green; only OpenBSD executing the assertion
  caught it.

- `net_parity.rs`'s three real-backend tests moved into one
  `#[cfg(any(…))]`-gated `mod bsd` rather than carrying three copies
  of a now-five-armed gate; `net_nonblocking.rs`'s file-level gate
  widened to match. Both must stay textually identical to `lib.rs`'s.
  `ios`/`tvos`/`watchos` are deliberately *not* in the gate: Darwin, so
  they would very likely work, but nothing in CI could verify them
  (RFC v2 §3).

### 0.20.0

- Added `platform_windows::{WindowsPty, WindowsPtyMaster}` (rustils#83),
  part 2/2 of the PTY surface (Phase 7, D13) — the Windows ConPTY
  backend for `platform::pty` (part 1/2, `0.19.0`). `CreatePseudoConsole`
  wired to the child at `CreateProcessW` time via
  `STARTUPINFOEXW`/`PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE` — the only way
  to attach a pseudo console at all. Not grouped (no Job Object) —
  `kill_tree` on a pty-hosted `Child` is `Unsupported` on Windows, a
  deliberate scope reduction rather than a settled design choice.
  `STARTUPINFOEXW.dwFlags` also sets `STARTF_USESTDHANDLES` (with null
  std handles): live CI testing found that without it, a *spawning*
  process whose own stdio is itself redirected — exactly `cargo test`
  under any CI runner — has the kernel duplicate its redirected handles
  into the child regardless of `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE`,
  bypassing the pseudo console entirely (a documented Windows
  console-handle-inheritance gap, per `microsoft/terminal` maintainer
  guidance in discussion #15814, not a bug in this crate's spawn
  sequence — which otherwise matches Microsoft's own ConPTY sample
  byte-for-byte). `read`/`write` are ordinary blocking `ReadFile`/
  `WriteFile` on ConPTY's two pipe handles. `Drop` does a bounded
  `PeekNamedPipe` drain before `ClosePseudoConsole`, avoiding a real
  deadlock (`ClosePseudoConsole` blocks until conhost's internal writer
  finishes, which can block against an un-drained pipe). One background
  thread *is* needed, though, for `PtyMaster::read`'s own portable
  `Ok(0)`-on-child-exit contract: unlike a Unix pty slave, which the
  kernel closes automatically once its last holder exits, ConPTY's
  output pipe stays open until `ClosePseudoConsole` runs — confirmed
  live (a child that had already exited still left reads blocked
  indefinitely). `spawn_exit_watcher` waits on a duplicated process
  handle and calls bare `ClosePseudoConsole` once the child exits,
  guarded by a shared `closed` flag against a double-close race with
  `Drop` (whichever happens first — the child exiting, or the caller
  dropping the master — wins). Deliberately does *not* drain the output
  pipe first the way `Drop`'s own close does: an earlier version did,
  and live CI caught the real problem with that — the watcher's own
  drain raced (and consistently won against) a caller's concurrent
  `read()` on the same handle, three previously-passing tests losing
  real output to it. Calling bare `ClosePseudoConsole` has no such race
  (this thread never touches the output pipe at all — any pending
  `ReadFile` unblocks naturally once conhost's write-side duplicate
  closes), at the cost of moving, not removing, the drain's original
  purpose: `ClosePseudoConsole` can itself block if conhost's writer is
  stuck behind a full unread pipe, which now stalls this background
  thread instead — acceptable since it's detached and never joined.
  New divergence (`docs/divergences.md`
  #011): a single pollable fd on Linux vs two non-pollable handles on
  Windows — `WindowsPtyMaster` exposes `input_handle`/`output_handle`
  rather than a single `AsHandle`/`AsRawHandle`. CI-verified only (no
  Windows execution available in the implementing session) —
  `platform-windows/tests/pty.rs`, including a dedicated test that drops
  an undrained master against a child producing far more output than a
  pipe's default buffer holds, to actually exercise the teardown fix
  rather than trust it by inspection. See `docs/behavior/pty.md` and
  `docs/design-discussion-pty.md` for the full contract and reasoning.
  **Breaking**: none — an entirely new backend for an already-landed
  trait; nothing existing changed shape. Bumps `y` per
  `docs/versioning.md` §2's "additive counts too" rule (new `pub`
  items).

### 0.19.0

- Added `platform::pty::{Pty, PtyMaster}` (rustils#82), part 1/2 of the
  PTY surface (Phase 7, D13) — built without a confirmed live consumer,
  the owner's explicit call, same posture `CredentialStore`/`Sandbox`'s
  confinement half were built under. One atomic `Pty::spawn(cmd, size)`
  opens a fresh pty pair and spawns `cmd` attached to its slave side —
  not a separate open/attach pair, since Windows's ConPTY structurally
  can't attach to an already-running process. `Ok(0)` at EOF, matching
  `File::read`/`Terminal::read_chunk`'s existing convention.
  `platform_linux::{LinuxPty, LinuxPtyMaster}`: real pty pair +
  `posix_spawn`-based attach — **not** raw `fork`+`TIOCSCTTY`
  (shh's own donor mechanism): `POSIX_SPAWN_SETSID` plus a file action
  that opens the slave by pathname reaches the identical session-
  leader-with-controlling-terminal outcome without reopening the
  async-signal-safety hazard `sys::spawn`'s `posix_spawn`-only design
  exists to close (raw `fork` stays parked behind its own separate,
  still-undecided roadmap decision). Live-verified against
  `/proc/<pid>/stat` kernel ground truth, not just a successful
  `posix_spawn` return. `LinuxPtyMaster` ships `AsFd`/`AsRawFd` on the
  concrete type (Net/Tun precedent). `platform_mock::{MockPty,
  MockPtyMaster}`: scriptable, not a real pty (mirrors `MockTun`).
  See `docs/behavior/pty.md` for the full contract,
  `docs/design-discussion-pty.md` for the design reasoning.
  **Breaking**: none — an entirely new module and new backend types;
  nothing existing changed shape. Bumps `y` per `docs/versioning.md`
  §2's "additive counts too" rule (new `pub` items).
- Windows (ConPTY) not yet landed — issue #83, part 2/2, split out from
  this release given its own real size.

### 0.18.0

- Added `platform_linux::sys::secret_service` (rustils#78) — the Secret
  Service API (`org.freedesktop.secrets`) over `sys::dbus`'s transport
  (rustils#77), part 3/3 of `CredentialStore` (Phase 6 item 2).
  `LinuxCredentialStore` now delegates to it in place of the #76 stub:
  `available()` opens a session, resolves the default collection via
  `ReadAlias`, and unlocks it if locked and unlockable
  non-interactively (no `Prompt` completion — this is a headless
  backend); `get`/`set` search/create items keyed on the
  `service`/`account` attribute pair. Stateless — a fresh D-Bus
  connection and Secret Service session per call, mirroring the
  Windows backend's fresh `CredWriteW`/`CredReadW` per call.
  Reachability failures (no session bus, no provider, no default
  collection, a collection that can't be unlocked headlessly) report
  `Unavailable` from `available()` and a real `Err` from `get`/`set` —
  never a silent `Ok(None)`/`Ok(())`, per the trait's own contract.
  Live-verified against a real `dbus-daemon --session` +
  `gnome-keyring-daemon --unlock --components=secrets` pair spawned as
  a CI test fixture (round-trip, per-account isolation, replace-on-set,
  binary payloads), the same bar #77's transport was held to — CI now
  also installs `gnome-keyring` alongside `dbus`.
  **Breaking**: none — `LinuxCredentialStore`'s trait impl signature is
  unchanged from #76; only its behavior moved from stub to real. Bumps
  `y` because `pub mod secret_service` under `sys` (itself `pub`) is a
  new public item, the same reasoning #77's `pub mod dbus` bump used.

### 0.17.0

- Added `platform_linux::sys::dbus` (rustils#77) — a hand-rolled D-Bus
  client transport, part 2/3 of `CredentialStore` (Phase 6 item 2): no
  existing D-Bus dependency, matching this repo's raw-bindings
  philosophy over the donor's `keyring-rs` wrapper. Little-endian
  message marshaling/unmarshaling for the type-system subset Secret
  Service needs (basic types, array, struct, variant, dict-entry),
  `AF_UNIX` session-bus connect (both real-path and Linux
  abstract-namespace addressing), the SASL `EXTERNAL` handshake, and
  the mandatory post-auth `Hello` registration call (missed on the
  first pass — every other call came back `AccessDenied` until this
  was added, caught by the live integration test, not a round-trip
  unit test). Internal to `platform-linux` only — no `platform::*`
  trait surface change, no `CredentialStore` behavior wired up yet
  (that's rustils#78, built on top of this).
  **Breaking**: none — `sys` is additive-only here, nothing existing
  changed shape (still bumps `y` per `docs/versioning.md` §2's
  "additive counts too" rule, since `pub mod sys` is real public
  surface even though no portable trait uses it yet). Live-verified
  against a real `dbus-daemon --session` spawned as a CI test fixture
  (new CI step: install `dbus` on the `ubuntu-latest` legs), not just
  unit tests — every wire-format alignment/padding rule is also
  asserted byte-for-byte in `wire.rs`'s own tests, not merely
  round-tripped.

### 0.16.0

- Added `platform::security::CredentialStore` (`get`/`set`/`available`)
  and `NullCredentialStore`, the Security surface's second slice (RFC
  v2 R5+, D15, Phase 6 item 2, rustils#76) — built without a confirmed
  live consumer, the owner's explicit call (same posture as `Sandbox`).
  Windows: real Credential Manager (`CredWriteW`/`CredReadW`,
  `CRED_TYPE_GENERIC`, `CRED_PERSIST_LOCAL_MACHINE`) — needed the new
  `Win32_Security_Credentials` `windows-sys` feature. `TargetName` is
  composed from both `service` and `account` (Credential Manager's
  identity key is `TargetName`+`Type` alone, not `UserName`, so two
  accounts under one service would otherwise clobber each other).
  Linux: an `Unsupported` stub for now — the real Secret Service
  implementation (`org.freedesktop.secrets` over a hand-rolled D-Bus
  client, no new dependency) is rustils#77/#78, tracked separately
  given the size. `platform-mock`: a faithful in-memory fake. No
  `delete` — not part of the roadmap's documented scope for this slice.
  **Breaking**: none — a wholly new trait, nothing existing changed
  shape (still bumps `y` per `docs/versioning.md` §2's "additive counts
  too" rule). Live-verified on Windows against real Credential Manager
  state. See `docs/behavior/security.md` for the full contract.

### 0.15.0

- Added `Spawner::adopt(pid) -> Result<Box<dyn GroupHandle>>` and a new
  `GroupHandle` trait (`kill_tree`/`kill_single` only — no `wait`/stdio,
  since an adopted pid was never spawned through this crate) — rustils#47,
  the "attach a Job Object to an externally-spawned pid" gap
  (`nexus-terminal`'s `JobObject::assign_pid`, for PTY sessions
  `portable-pty` spawns rather than this crate). Windows: `OpenProcess`
  + a fresh kill-on-close Job Object (`AssignProcessToJobObject`) — the
  same mechanism `GroupSpec::NewGroup` uses at spawn time, applied after
  the fact. Unix: always `Unsupported` (`docs/divergences.md` #010) —
  POSIX `setpgid(pid, pgid)` can only retarget the caller's own
  not-yet-exec'd child, never true by the time a caller has a pid to
  adopt, so this is a genuine one-directional OS capability gap, not
  attempted speculatively. `platform-mock`: succeeds unconditionally
  (no OS process to fail against), logging calls to the new
  `MockSpawner::adopted` field.
  **Breaking**: new required `Spawner` method — breaking for any
  external `Spawner` implementer (none outside this repo's own three
  backends exist yet). Live-verified on Windows: `kill_tree` on the
  *adopted* handle reaches the *original* spawned child, proving
  `AssignProcessToJobObject` landed on the real process. See
  `docs/behavior/process.md` and `docs/extraction-map.md`'s D2 landed
  note for the full contract.

### 0.14.0

- Added `Dir::set_unix_mode` and a new `Mode` struct
  (`setuid`/`setgid`/`sticky`/`permissions`, no `uid`/`gid` — that's
  `chown`'s job) — coreutils gap backlog #64, `unix_mode`'s write-side
  companion (`fchmodat`-equivalent). Linux: `fchmodat(dirfd, rel, mode,
  0)`, following a terminal symlink (the kernel has no symlink-mode
  concept to target, matching `chmod(1)`'s own behavior). Windows:
  `Err(Unsupported)` (`docs/divergences.md` #009) — never a silent
  no-op, since the caller's entire ask was to change permissions.
  `platform-mock`: accepts the call (`NotFound` still enforced on a
  missing entry) without persisting anything, matching `unix_mode`'s
  own fixed-default stance. Track P: also `Unsupported` for now —
  `rusty_libc` has no `chmod`/`fchmodat` primitive at the pinned rev.
  Landed ahead of a named `coreutils` consumer (no `rchmod` exists) —
  see `docs/coreutils-gap-backlog.md`'s Gap 3 resolution note.
  **Breaking**: new required trait method — breaking for any external
  `Dir` implementer (none outside this repo's own three backends
  exist yet). Live-verified on Linux against a raw `libc::stat` call.
  See `docs/behavior/fs.md` and the convergence roadmap for the full
  contract.

### 0.13.0

- Added `Metadata::nlink: u64`/`modified: SystemTime` and
  `UnixMode::permissions: u16` (coreutils gap backlog #63/#64/#65) —
  forced by this repo's own `coreutils::ls -l` reference consumer, the
  `ls -l` donor material. `nlink`/`modified` are portable (both
  backends genuinely have a link count and mtime, no `Option` needed);
  `permissions` is the standard `rwxrwxrwx` bits, read-only at the time
  — the `chmod`-equivalent write path landed separately in 0.14.0.
  **Breaking**: both are new required fields on existing public
  structs — breaking for any external construction of `Metadata`/
  `UnixMode` (none outside this repo's own three backends and
  `platform-mock` exist yet). Live-verified per backend against a
  second, independent source (Linux: raw `libc::stat`; Windows:
  `std::fs::Metadata::modified()` + a raw
  `GetFileInformationByHandleEx(FileStandardInfo, ...)` call). See
  `docs/behavior/fs.md` and the convergence roadmap's Phase 3 entry
  for the full contract and backend notes.
- Added `platform_linux::{user_name, group_name}` (`getpwuid_r`/
  `getgrgid_r`) — uid/gid → display-name resolution backing
  `coreutils::native`'s `-l` output, deliberately **not** part of
  `platform::fs`/`Dir`/`UnixMode` (a directory-service lookup, not
  filesystem metadata). Linux-only; nothing to resolve on Windows
  (`Dir::unix_mode` is always `None` there).

### 0.12.0

- Added a raw-socket + non-blocking escape hatch to `platform-windows`'s
  concrete Net socket types (`WindowsTcpStream`/`WindowsTcpListener`/
  `WindowsUnixStream`/`WindowsUnixListener`/`WindowsUdpSocket`)
  (rustils#59) — the `platform-windows` half of the gap rustils#41 left
  on Linux. Forced by `rusty_tail`'s `rusty_tokio` hand-rolled async
  runtime scoping a Windows/IOCP reactor backend (`rusty_tokio#6`), the
  same consumer #41/#48 already served on Linux/macOS. Adds
  `AsRawSocket` (raw-handle exposure only, delegating to the private
  `sysnet::OwnedSocket`), `set_nonblocking` (`ioctlsocket(FIONBIO,
  ...)`), and concrete `connect`/`bind`/`accept` constructors returning
  the concrete type directly instead of `Box<dyn Trait>` (`Net`'s own
  trait methods are now thin wrappers over these, mirroring the Linux
  slice exactly). No `AsSocket`/ownership-transfer interop — this
  crate's `OwnedSocket` is its own newtype, not std's
  `std::os::windows::io::OwnedSocket`, and nothing has asked for
  adopting an externally-created socket on Windows the way
  `From<OwnedFd>` does for Unix. See the convergence roadmap's Phase 5
  entry for the full backend notes.

### 0.11.0

- Added the Tun / virtual-link surface (D14, convergence roadmap Phase
  8): `platform::tun::{Tun, TunDevice}`, forced by rusty_tail's
  `ts-tun`, the single named consumer. `Tun::create(name, ipv4,
  prefix_len, mtu)` bundles device creation, IPv4/prefix addressing
  (which installs the connected route), MTU, and bring-up into one
  call, mirroring `ts-tun/src/sys.rs`'s own hand-rolled ioctl sequence
  exactly. Linux: `/dev/net/tun` + `TUNSETIFF`, then
  `SIOCSIFADDR`/`SIOCSIFNETMASK`/`SIOCSIFMTU`/flags-up over a throwaway
  `AF_INET`/`SOCK_DGRAM` socket — live-verified against a real kernel
  (real interface, real installed route, a real kernel-routed outbound
  packet, and a hand-crafted checksummed inbound packet delivered to a
  bound `UdpSocket`), not merely cross-compile-checked.
- The concrete `platform_linux::LinuxTunDevice` additionally exposes
  `AsFd`/`AsRawFd`/`set_nonblocking` on the concrete (non-boxed) type —
  the same raw-fd escape hatch rustils#41/#42 established for `Net`,
  since `ts-tun` needs to register the device's fd with tokio's own
  reactor directly, exactly as `ts-magicsock` did onto
  `platform_linux::LinuxUdpSocket`.
- `platform_windows::WindowsTun::create` reports `ErrorKind::Unsupported`
  explicitly rather than the module being absent — no Windows consumer
  has named itself (`ts-tun` is `#![cfg(target_os = "linux")]` only), so
  there is no donor evidence for a `wintun`-backed shape yet. No
  `platform-macos` `Tun` impl exists at all — same "no consumer, no
  speculative surface" call.
- Added `platform_mock::{MockTun, MockTunDevice}`: does not simulate
  kernel routing (unlike `MockUdpSocket`/`MockTcpStream`, there is no
  peer-socket "other side" to fake for a TUN device — the real
  counterpart is the kernel's own routing table). Scriptable instead:
  `MockTunDevice::queue_inbound` queues bytes for a future `read()`,
  and `written_packets()` returns everything recorded via `write()`.
  Does not block on an empty queue (`read()` returns `Ok(0)`
  immediately) — no real mechanism to block on, the same tradeoff
  `MockCsprng` makes for randomness quality.
- See `docs/behavior/tun.md` for the full behavior contract.

### 0.10.0

- Added `Stdio::File(Box<dyn platform::fs::File>)` (D5, rustils#51):
  wires a spawned child's stdin/stdout/stderr to an already-open `File`
  — the `> file`/`>> file`/`< file`/`2>&1`/`&> file` shell-redirect
  shapes `nexus-rush/src/exec.rs::build_stage` needs, filed as a direct
  follow-up once #43–#46 landed and converting `job.rs`'s
  `spawn_pipeline` onto `Spawner::spawn` hit this gap. Mechanism only:
  a spawn-time `dup2`/`DuplicateHandle`-style wiring that borrows rather
  than consumes the caller's `File`. `Spawner::spawn` fails
  `Unsupported` for a `Stdio::File` value from a different backend.
- Added `File::try_clone(&self) -> Result<Box<dyn File>>` (`dup(2)`/
  `DuplicateHandle`, shared open-file-description including position) —
  the `2>&1`/`&> file` half of the same redirect shape: two
  `Stdio::File` slots need to share one file's position, which two
  independent `Dir::open` calls on the same path cannot give them.
  Also added `File::as_any(&self) -> &dyn Any`, a downcast hook mirroring
  `Child::as_any_mut` that a backend's `Spawner::spawn` needs to recover
  its own concrete `File` type from a `Stdio::File`'s object-safe
  `Box<dyn File>`. Both are **new required methods on an existing
  trait** — breaking for any `File` implementor (none outside this
  repo's own three backends exist yet).
- **Breaking**: `Stdio` is no longer `Copy`/`Clone`/`PartialEq`/`Eq`,
  and `Command` is no longer `Clone` — a `Stdio::File` slot owns an
  open OS handle with no honest value-type-copy meaning. Callers that
  compared `Stdio` with `==` need `matches!` instead (the only such
  caller in this repo, `platform-mock`, was updated).
- **Breaking** (`platform-mock` only): `MockSpawner::spawned`'s element
  type changed from `Command` to a new `SpawnRecord` struct (with a new
  `StdioKind` enum for its `stdin`/`stdout`/`stderr` fields) — the
  direct consequence of `Command` losing `Clone`; existing field-name
  reads (`spawned[0].cwd`, etc.) are source-compatible.
- Per `docs/versioning.md` §2, all of the above land in one `y`-bump
  regardless of which parts are additive vs. breaking, same rule as
  every prior entry here.

### 0.9.0

- Added the job-control slice (rustils#43–#46), converging
  `platform::process`/`platform::term` onto what `nexus-rush/src/job.rs`
  needs (`baileyrd/nexus#454`): `GroupSpec::JoinGroup(pgid)` (join an
  existing process group at spawn, D1's pipeline shape); a portable
  `Signal` enum (`Term`/`Int`/`Hup`/`Quit`/`Kill`/`Stop`/`Cont`) —
  `Child::kill_tree`/`kill_single` now take a `Signal` instead of a
  hardcoded `SIGKILL`; `ExitStatus::Stopped`/`Continued` plus
  `Child::wait_job`/`try_wait_job` (D10, the `WUNTRACED`/`WCONTINUED`
  half of wait); and `platform::term::JobControlTerminal::give_terminal`
  (`tcsetpgrp`), a new Unix-only extension trait implemented only by
  `LinuxTerminal`. Breaking for existing `Child` implementers
  (`kill_tree`/`kill_single`'s signature changed, two new required
  methods) — per `docs/versioning.md` §2 this is a `y`-bump regardless
  of the additive/breaking split, same as `TcpStream::set_read_timeout`
  was. Windows gains divergence-registry entry **008** for what it
  can't do (only `Signal::Kill`; no `GroupSpec::JoinGroup`; no
  `wait_job`/`try_wait_job`). This bump was missed at merge time and is
  being recorded after the fact — no functional change since #49
  landed, just the version/changelog catching up to it.
- `platform-macos` joined the PAL group (rustils#48): a net-only
  backend (`Net`/`TcpStream`/`TcpListener`/`UnixStream`/
  `UnixListener`/`UdpSocket`, plus the rustils#41 `AsFd`/`AsRawFd`/
  `From<OwnedFd>`/`set_nonblocking`/concrete-constructor surface from
  day one), forced by `rusty_tail`'s `rusty_tokio` hand-rolling the
  same socket lifecycle a second time for its macOS/BSD kqueue reactor.
  No change to any existing crate's public API shape — a new
  implementor joining the group's existing `platform::net` traits, not
  a trait-shape change — so this entry is bookkeeping (which
  `platform` this `platform-macos` build implements), not the reason
  for this bump; see the job-control entry above for that. Not yet run
  against real macOS hardware by this workspace's own CI — validated
  via `cargo check`/`clippy --target x86_64-apple-darwin`. See
  `docs/behavior/net.md` and the convergence roadmap's Phase 5 entry
  for the full contract and backend notes.

### 0.9.0

- Job-control slice (rustils#43/#44/#45/#46), forced by `nexus-rush/src/job.rs`
  (`baileyrd/nexus`, converging onto `platform::process`/`platform::term` per
  `baileyrd/nexus#454`):
  - `GroupSpec::JoinGroup(pgid)` — a pipeline stage joins an existing process
    group at spawn (race-free, same as `NewGroup`) instead of leading a fresh
    one. Unix only; `Unsupported` on Windows.
  - `Child::kill_tree`/`kill_single` now take a `Signal` parameter (**breaking**
    — previously no argument, always `SIGKILL`) — a new portable `Signal` enum
    (`Term`/`Int`/`Hup`/`Quit`/`Kill`/`Stop`/`Cont`). Windows accepts only
    `Signal::Kill`; every other variant is `Unsupported` there.
  - `ExitStatus::Stopped(sig)`/`Continued` plus `Child::wait_job`/`try_wait_job`
    (`WUNTRACED`/`WCONTINUED`) — the Ctrl-Z/`fg`/`bg` half of wait. Unix only;
    `Unsupported` on Windows.
  - `platform::term::JobControlTerminal` — a new, separate trait (not folded
    into `Terminal`) providing `give_terminal(pgid)` (`tcsetpgrp`), encoding
    the `SIGTTOU`-ignored precondition into every call. Implemented only by
    `LinuxTerminal` — no Windows equivalent exists to implement it, which is
    exactly why it's its own trait rather than a `Terminal` method every
    backend would have to answer for.
  - New divergence-registry entry 008 records the Windows-side gaps (no
    general signal delivery, no numeric-pgid group join).
  - rustils#47 (Windows: adopt an externally-spawned pid into a Job Object)
    deliberately did **not** get an API here — no forcing consumer yet
    (`JobObject::assign_pid` is dead code in `nexus`) — left open as a
    recorded gap per RFC v2 §3's consumer gate.

### 0.8.0

- Added a raw-fd + non-blocking escape hatch to `platform-linux`'s
  concrete Net socket types (`LinuxTcpStream`/`LinuxTcpListener`/
  `LinuxUnixStream`/`LinuxUnixListener`/`LinuxUdpSocket`): `AsFd`,
  `AsRawFd`, `From<OwnedFd>`, and `set_nonblocking` — plus concrete
  `connect`/`bind`/`accept` constructors returning the concrete type
  directly instead of `Box<dyn Trait>` (`Net`'s own trait methods are
  now thin wrappers over these). Forced by rustils#41: rusty_tail's
  `rusty_tokio` hand-rolled async runtime wants to register a socket
  with its own reactor rather than reimplement socket setup from
  scratch. Inherent-impl-only — the object-safe `platform::net` traits
  are unchanged, matching `LinuxFile`/`LinuxDir`'s existing std-interop
  precedent (`fs.rs`). Linux only; not part of the cross-backend
  `docs/behavior/net.md` spec.

### 0.7.0

- Added the Security surface's third slice: `platform::security::Sandbox`
  (`confine_filesystem` via raw Landlock syscalls, `block_inet_sockets`
  via a hand-written seccomp-BPF filter), mirroring nexus's
  `os_sandbox.rs` shape exactly. Built without a confirmed live
  consumer, an explicit owner call made after an RFC-level design
  discussion (`docs/design-discussion-sandbox.md`) found nexus's and
  shh's donor material solve two different problems — process
  confinement vs. privilege-separation isolation — that don't share a
  trait shape; only the confinement half landed. `CredentialStore`
  (the middle slice) stayed held on the same trip: nexus's existing
  `CredentialVault` has no live gap to converge on. `x86_64`/Linux
  only for now; every other backend reports `SandboxStatus::Unsupported`
  rather than silently claiming enforcement.

### 0.6.0

- Added the Security surface's first slice: `platform::security::Csprng`,
  `fill_random`, forced by rusty_rdp's five hand-rolled `/dev/urandom`
  reads (`src/krb5/kdc.rs`). Deliberately narrow — one method, no key
  derivation, no algorithm choice. Linux draws from the raw
  `getrandom(2)` syscall, Windows from `BCryptGenRandom` with the system
  preferred RNG — neither opens `/dev/urandom` as a file, avoiding an
  `fd` a later filesystem sandbox policy (this same Phase 6's largest
  remaining slice) might otherwise deny.

### 0.5.0

- Added `TcpStream::set_read_timeout` — an idle read timeout, forced
  by a real gap found while starting the rusty_rdp convergence
  (`examples/connect.rs` needs it; `platform::net::TcpStream` had no
  equivalent). Scoped to `TcpStream` only (RFC v2 §3 — no consumer
  has asked for it on `UnixStream`/`UdpSocket` yet).
- (Test-only, no version bump on its own, noted here for context:) a
  real pre-existing race in the Unix-socket parity suite was found and
  fixed in the same PR — unrelated to the timeout addition itself.

### 0.4.0

- Added the UDP datagram slice: `Net::udp_bind`, `UdpSocket`
  (`send_to`/`recv_from`/`local_addr`), completing D16's three-slice
  survey (TCP, Unix sockets, UDP) named for rusty_tail's magicsock.
- Unix-socket parity suite landed in a follow-on PR — test-only, no
  bump of its own.

### 0.3.0

- Added the Unix domain socket slice: `Net::unix_connect`/
  `unix_listen`, `UnixStream`, `UnixListener` — mode-`0600` bind and
  automatic stale-cleanup bind (a throwaway probe `connect`
  distinguishes a dead listener's leftover socket file from a live
  one). An early pass of this slice shipped with the wrong
  stale-cleanup contract (caller-must-unlink-first); caught and
  corrected before merge, so it never shipped under a version number.

### 0.2.0

- Added the TCP slice: `Net`, `TcpStream`, `TcpListener` — the first
  half of the Net surface (RFC v2 R5+, D16), named for shh, rusty_tail,
  rusty_rdp, and rusty_llama's optional server. No TLS concept in the
  trait; all four named consumers bring or inject their own wire
  crypto.

### 0.1.0 and everything before this changelog existed

Everything from the initial extraction through Track P completion:
`Fs` (capability `Dir`/`File`, byte `OsStr` boundary), `Process`
(`Command`/`Spawner`/`Child`, decoded `ExitStatus`, groups/
`kill_tree`, pipes), `Events` (deferred `SignalSource`, multiplexed
`wait_any`), the two-axis error model, the parity regime
(`platform-mock` as the third backend, the divergence registry), and
Track P (the `rusty_libc` raw-syscall floor behind the `track-p`
feature). See `docs/convergence-roadmap.md`'s Phase 1–4 entries and
`docs/extraction-map.md` for the real per-decision history — this
changelog doesn't re-derive it.

## `winargv`

### 0.1.0

Versioned independently from the PAL group starting here (previously
shared the workspace version by accident of `version.workspace = true`,
not by any real coupling — see `docs/versioning.md` §1). No functional
change in this bump; MSVCRT/cmd-rules quoting and refuse-unrepresentable
were already complete and fuzz-hardened before this changelog existed.

## `coreutils`

### 0.1.0

Versioned independently from the PAL group starting here, for the same
reason as `winargv` above — no functional change in this bump.
`coreutils` is an internal reference-consumer (RFC v2 §3); nothing
outside this repo depends on it, so its version has no audience beyond
this repo's own history.
