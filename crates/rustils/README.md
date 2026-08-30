# rustils

A hand-rolled, Rust-native platform personality layer for Windows and
Linux: strongly-typed, capability-style APIs over the NT and Linux kernels,
built above raw bindings (`windows-sys`, `libc`) with all `unsafe` confined
to audited backend `sys/` modules.

**Governing document: [`docs/rfc-v2.md`](docs/rfc-v2.md).** Read it before
adding anything — in particular §3 (the consumer gate: no API without a
named, working consumer) and §5 (binding API design requirements).

## Dual mandate

- **M1 — Understanding.** This project exists partly to learn the machine.
  Hand-rolling is a feature; each hard-won lesson lands as a note in
  [`docs/learning/`](docs/learning/).
- **M2 — Foundation.** The layer must be real: consumed by working
  software, parity-tested across OSes, and stable enough to build on.
  First external consumer under contract: the **rush** shell
  (RFC §7 — mechanisms hoist here at rush's Phase 2 gate).
  **Status note (2026-08-10):** this is a forward commitment, not a
  present fact — rush's `Cargo.toml` today declares no dependency on
  `platform`/`rustils` at all; its job control calls `rusty_libc`/
  `rusty_win32` directly rather than going through `Process`/`Events`.
  "First external consumer" describes what Phase 2 hoists, not rush's
  current dependency graph (see #117).

## Layout

```
crates/
  platform          portable traits + types (api layer; forbid(unsafe))
  platform-mock     in-memory backend — the injectable test double
  platform-linux    libc floor; ffi (curated surface) → sys (all unsafe) → impls
  platform-windows  windows-sys floor (+ rusty_win32 track-w); same layering
  winargv           MSVCRT + cmd-rules command-line quoting — standalone for handback
  coreutils         reference consumer (rcat, ls) — tested against the mock
docs/
  rfc-v2.md              the governing RFC
  architecture.md        target-state layer map + ecosystem repo placement
  convergence-roadmap.md phased migration/convergence sequencing
  behavior/              per-API behavior specs the parity suite asserts
  divergences.md         numbered cross-backend divergence registry
  learning/              M1 write-ups
  rusty-mill-profile.md  Draft Rusty-Mill ecosystem standards profile (see below)
```

## Rusty-Mill ecosystem membership

rustils is pursuing membership in the [Rusty-Mill](https://github.com/Rusty-Mill/rusty_foundation_akb)
capability-based platform ecosystem. [`docs/rusty-mill-profile.md`](docs/rusty-mill-profile.md)
is **Accepted**, scoped narrowly to the **filesystem** domain, which
itself holds an accepted Experimental promotion decision in the AKB.
Process, networking, and security are proposed but not yet bound — each
needs its own accepted domain promotion and its own profile-scope
widening, not inferred from filesystem's. An implementation trial
(`TRIAL-0002`) citing this repository as evidence exists but remains
**Not authorized** (Cross-cutting and independent-review gates still
open). Outside filesystem, rustils makes no Rusty-Mill conformance or
maturity claim and continues operating under its own RFC below.

## Verify

```
cargo test --workspace     # unit + mock + parity (mock and native backends)
cargo build && ./target/debug/rcat some-file
```

## Status

Release **R0/R1 (partial)** per the RFC roadmap: workspace, error model,
capability-fs trait surface, mock backend, Linux `Dir` over the `openat`
family, Windows `Dir` over `NtCreateFile` handle-relative opens (the
ntdll admission rationale lives in `platform-windows/src/ffi/nt_surface.rs`),
parity suite on both OS legs, std-interop on all handle types (RFC §5.1),
reference consumers (`rcat`, `rls`) wired to both native backends, CI
(fmt, clippy `-D warnings`, tests on ubuntu+windows × stable+MSRV, mingw
cross-compile pre-check, Miri on the pure crates, unsafe-scope gate,
cargo-deny). Process semantics are specced
([`docs/behavior/process.md`](docs/behavior/process.md)) with the mock as
the anchor; the native spawn/quoting/groups/reactor mechanisms are ported
from rush and its satellite crates per the extraction map
([`docs/extraction-map.md`](docs/extraction-map.md), RFC §7 Amendment A1)
— proven donors mined deliberately, not designed here from scratch.
Extraction step 1 is in: `winargv` (MSVCRT quoting + cmd-rules batch
quoting with refuse-unrepresentable — closes the BatBadBut class),
oracle-tested against `CommandLineToArgvW` on the Windows leg. Step 2 is in: native `Spawner`/`Child` on both OSes (`posix_spawn`;
`CreateProcessW` over `winargv`), decoded `ExitStatus` parity-pinned,
`rrun` as the consumer, and first-class groups — `GroupSpec::NewGroup`
with `kill_tree`/`kill_single` (`setpgid`-at-spawn; suspended-spawn into
a kill-on-close Job Object), with the registry's first divergence
entries (001/002) recorded. Step 3 and R3 are in: `Child::try_wait`, `wait_any` (portable +
multiplexed — pidfd+`poll` / `WaitForMultipleObjects` with the
64-handle cap absorbed), and the D6 `SignalSource` (deferred, one-store
handlers; console-ctrl mapping on Windows, divergence 003) — `rpar`
assembles the full §5.6 reactor from them. Step 4 is
in: `Stdio::Pipe` capture/feeding with inheritance control on every
backend (consumed by `rtee`), with the STARTUPINFO-vs-slot-swap
decision recorded in the extraction map. Linux Track P (raw syscalls
behind the `track-p` feature, via a pinned `rusty_libc` dependency,
D-12) covers every migrated family in `platform-linux/src/sys`: fdio
(read/write, the openat family, statx), the reactor's pipe2/poll, and
process control (kill, wait4, the signal trampoline) — parity-verified
in both configurations on every CI run. Windows has the counterpart
move as of D-15: **Track W**, a pinned `rusty_win32` dependency behind
the off-by-default `track-w` feature, migrating call families off
windows-sys one at a time (`sys::fileio::read`/`write` first) and
re-running the whole platform-windows suite in that configuration as
the equivalence test. Unlike Track P it buys no lower tier — Windows
publishes nothing supported beneath a documented DLL export, so it
swaps the binding's provenance rather than its depth, and windows-sys
stays the default floor (`docs/learning/003-…`). A full-ecosystem donor survey
(`docs/extraction-map.md` D9–D16) then unparked the **Terminal**
surface: is_tty, window size, and raw-mode enter/leave over termios
(Linux) and console modes (Windows), with `rusty_term` as the design
oracle. Convergence roadmap Phase 2 (`docs/convergence-roadmap.md`)
added slice 2 — a live `is_raw` probe, `poll_readable`/`read_chunk`,
and `set_echo` — all consumed and live-verified by `rterm`, with
bracketed paste and suspend/resume deliberately needing no further
surface (they're already expressible with what landed). PTY hosting,
resize-notification, and job-control handoff remain gated follow-ons.
Phase 3 grew the `Fs` surface (D11): `File::sync_all`,
`Dir::rename`/`rename_no_replace` (Linux `renameat2`; Windows
handle-relative `FILE_RENAME_INFORMATION` via `NtSetInformationFile`),
and a default-provided `Dir::write_atomic` composed from both —
strace-verified to fsync before it publishes. A follow-on slice added
`Dir::symlink`/`read_link` (Linux `symlinkat`/`readlinkat`; Windows
`FSCTL_SET_REPARSE_POINT`/`FSCTL_GET_REPARSE_POINT` over a hand-built
`REPARSE_DATA_BUFFER`), with the one thing Windows requires that POSIX
doesn't — declaring file-vs-directory at creation — registered as a
divergence rather than papered over (`docs/divergences.md` #004). A
further slice added `Dir::access` (`faccessat`, real not effective
uid/gid on Linux; a trial open on Windows): Windows has no
execute-permission bit on a regular file at all, so `execute` is
granted unconditionally once existence is confirmed, pinned as a second
registered divergence (`docs/divergences.md` #005) with dedicated
backend-only tests rather than a forced-uniform assertion. A final
slice rounded out `test`'s donor predicates with `Dir::unix_mode`
(`-u/-g/-k/-O/-G`, real mode bits + ownership on Linux, honest `None` —
not fabricated — on Windows) and `Dir::file_id` (`-ef`, an opaque
same-file identity every backend answers identically). The
PATH-resolution half of that donor item turned out to already exist as
`Spawner::resolve`; what's left is ecosystem-side (rush adopting it),
out of scope here.

Phase 4 (Track P completion) closed platform-linux's last two
raw-syscall gaps: `rusty_libc` gained `getdents64`/`dirents` and
`pidfd_open`/`P_PIDFD` (`rusty_libc` PR #19), and here, `read_dir`'s
`track-p` arm now calls `getdents64` directly instead of glibc's
`fdopendir`/`readdir` `DIR*` stream (which has no raw-syscall
equivalent of its own to reimplement), and `poll_pids`'s pidfd-opening
step lost its raw `c::syscall` escape hatch under `track-p` — it's a
real wrapper call now, live-verified via strace (a real `read_dir`
firing `getdents64`, a real two-child `wait_any` firing `pidfd_open`
for each pid).

Phase 5 landed the `Net` surface (D16), all three named slices. TCP
shipped `Net`, `TcpStream`, `TcpListener` for
connect/listen/accept/`set_nodelay`. Unix domain stream sockets added
`Net::unix_connect`/`unix_listen`, `UnixStream`, `UnixListener` —
mode-`0600` bind and automatic stale-socket-file cleanup (a throwaway
probe connect tells a dead listener's leftover file apart from a live
one; Linux narrows via `chmod`, a registered divergence since Windows'
`AF_UNIX` bind has no mode-bit equivalent to narrow —
`docs/divergences.md` #007). UDP datagram sockets closed the surface
out with `Net::udp_bind`/`UdpSocket` for rusty_tail's magicsock — no
listener/stream split, one connectionless socket addressed per call via
`send_to`/`recv_from`, and the one genuinely new behavior across the
whole surface: `send_to` never fails just because nothing is bound at
the destination (no handshake to fail the way TCP/Unix connect have),
strace-verified on a real closed-socket send. No TLS concept anywhere
in any slice — the four named consumers (shh, rusty_tail, rusty_rdp,
rusty_llama's optional server) all bring or inject their own wire
crypto. Linux uses raw `libc` socket calls, not track-p-gated (sockets
were never in rush's required surface, so there's nothing to route
through `rusty_libc` — `fsync`'s precedent); Windows uses raw Winsock2
with a lazily-started, deliberately never-cleaned-up `WSAStartup`
(matching mio/tokio/std's own Windows networking); the mock backend is
an in-memory implementation with real
connection-refused/addr-in-use/end-of-stream/fire-and-forget semantics.
Strace-verified on Linux throughout. See `docs/behavior/net.md`.

Phase 6 built out the `Security` surface (D15). First slice:
`platform::security::Csprng::fill_random`, forced by rusty_rdp's five
hand-rolled `/dev/urandom` reads (all now retired, in both `krb5/kdc.rs`
and `tls.rs`). Deliberately narrow — one method, no key derivation.
Linux draws from the raw `getrandom(2)` syscall (strace-verified),
Windows from `BCryptGenRandom` with the system preferred RNG — neither
opens `/dev/urandom` as a file, so `Sandbox` confinement has no `fd` to
have denied.

Third slice: `platform::security::Sandbox` (`confine_filesystem` via raw
Landlock syscalls, `block_inet_sockets` via a hand-written seccomp-BPF
filter), built without a confirmed live consumer as an explicit owner
call after an RFC-level design discussion (`docs/design-discussion-sandbox.md`)
found nexus's and shh's "sandbox" material solve two different problems
that don't share a trait shape — only the confinement half (nexus's
shape) landed; shh's privilege-separation pattern doesn't fit
`platform::process`'s current shape and stayed out of scope.
`CredentialStore` (the middle slice) stayed held: nexus's existing
`CredentialVault` has no live gap to converge on. `block_inet_sockets`
is fully strace/live-verified (`x86_64` seccomp-BPF blocks
`AF_INET`/`AF_INET6`, leaves `AF_UNIX` untouched); `confine_filesystem`'s
Landlock syscall arguments are verified byte-correct but full kernel
enforcement wasn't exercisable in this session's own sandboxed
environment (`ENOSYS`, confirmed as an environment limitation via a
raw C probe, not an implementation bug). See `docs/behavior/security.md`.

Starting the rusty_rdp convergence (the roadmap's flagged cheapest
proof of the trait) surfaced one real gap in this "done" surface
before any code changed in rusty_rdp's own repo: `TcpStream` had no
read-timeout capability, which rusty_rdp's own example code needs.
`TcpStream::set_read_timeout` closes it — an idle timeout (`None`
blocks indefinitely, unchanged default), backend-chosen
`WouldBlock`/`TimedOut` on expiry (the same ambiguity
`std::net::TcpStream::set_read_timeout` itself has, not resolved here
either), strace-verified on a real 100ms timeout. Scoped to
`TcpStream` only — no named consumer needs it on `UnixStream`/
`UdpSocket` yet.

`Command::detach` + `Spawner::is_alive`/`is_zombie`
(`docs/decision-request-detach-liveness.md`) landed 2026-08-11 against
an external brief for a not-yet-started daemon-backed agent-harness
consumer, ahead of the RFC v2 §3 consumer gate by explicit owner
override — most of the brief (process-group-or-single kill, a
cross-transport local-IPC listener) turned out to already be shipped
surface (`Child::kill_tree`/`kill_single`/`GroupHandle`;
`Net::unix_connect`/`unix_listen` over native Winsock `AF_UNIX`, not
named pipes) and was not rebuilt. The two genuine gaps: a liveness/
zombie probe decoupled from `try_wait`'s `Child`-ownership model, and a
spawn-time `detach()` flag (`POSIX_SPAWN_SETSID` on Linux;
`CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS` on Windows). `detach()`
composes only with `GroupSpec::Inherit`; `NewGroup` is refused on
**both** backends, for two unrelated real reasons caught by CI rather
than assumed: Linux — `setsid` always makes the child a session leader,
and the kernel's `setpgid(2)` forbids changing a session leader's own
process group, so the combination fails `posix_spawn` itself with
`EPERM`; Windows — a kill-on-close Job Object would silently defeat
`detach`'s "survives a crash" guarantee. `is_zombie` is `Unsupported`
on Windows, which has no zombie concept (divergence 015). Every line of
the CI matrix is green (`fmt`, `clippy -D warnings` on every target
including the `track-w` leg, `test` on ubuntu/windows × stable/1.75,
miri, cross-compile, `cargo-deny`, unsafe-scope) — two real bugs this
Windows-sandboxed authoring session could not see locally (a
`platform-linux`-only `E0282`/`too_many_arguments`, invisible because
that crate's root is `cfg(target_os = "linux")` and no-ops under
`cargo check` on Windows; a pid-reuse race in a Windows liveness test,
caused by testing against a pid whose owning handle the consuming
`Child::wait` had already closed) surfaced and were fixed against real
CI feedback, not assumed away.

## License

MIT — matching the sibling crates (`rush`, `rusty_win32`, `rusty_libc`,
`rusty_lines`) so code flows both directions (extraction in, handback
out) under one license. See [`LICENSE`](LICENSE).
