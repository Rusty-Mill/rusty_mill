# Cross-Backend Divergence Registry

Numbered, append-only. Each entry: behavior per backend, the OS limitation
forcing it, the test pinning it, and the review that accepted it. Rule
(RFC v2 §9): a divergence may cite only an OS limitation, never
implementation convenience.

## 001 — status of a killed child

- **Linux**: `Child::kill_tree`/`kill_single` deliver `SIGKILL`; the
  subsequent `wait` reports `ExitStatus::Signaled(9)`.
- **Windows**: termination is `TerminateJobObject`/`TerminateProcess`
  with a caller-chosen exit code (this backend passes 1); `wait` reports
  `ExitStatus::Code(1)`.
- **OS limitation**: Windows has no signal concept — a terminated
  process's only observable is its exit code, and no code value is
  reserved to mean "killed". Synthesizing `Signaled` (or a 128+9-style
  code) on Windows would fabricate a mechanism the OS does not have.
- **Pinning tests**: `linux_process_group_kill` /
  `windows_process_group_kill` in each backend's `tests/parity.rs`.
- **Accepted**: 2026-07-19, with the groups/kill-tree extraction slice
  (extraction map D2/D8; rush's `winjob` reports `128+15` for its own
  kills — that is shell policy layered on this same mechanism, not a
  contradiction).

## 002 — dropping an un-waited `NewGroup` child

- **Linux**: the process keeps running; it is reparented and reaped by
  init if never waited (a leaked pid, nothing more).
- **Windows**: the child's Job Object is kill-on-close and the `Child`
  owns the only handle — dropping it terminates the whole tree.
- **OS limitation**: a Windows Job with kill-on-close is the only
  primitive that makes `kill_tree` reach grandchildren reliably; the
  close-at-drop side effect is inseparable from holding that guarantee.
  (rush's `disown` lesson — extraction map D2 — is the reversal
  mechanism, deliberately deferred until a consumer needs detach.)
- **Pinning tests**: the Windows behavior is exercised implicitly by
  every grouped parity test's drop path; an explicit survive-vs-die pin
  arrives with the detach API (whose absence is what makes an explicit
  test of the current behavior redundant with 001's kill test).
- **Accepted**: 2026-07-19, same slice.

## 003 — signal identities are console control events on Windows

- **Linux**: `SignalSource` events are real signals — `SIGINT`,
  `SIGTERM`, `SIGHUP` — delivered to any process.
- **Windows**: the deliverable identities are console control events
  (`CTRL_C_EVENT` → Interrupt, `CTRL_BREAK_EVENT` → Terminate,
  `CTRL_CLOSE_EVENT` → Hangup), delivered only to console processes; a
  detached or service process receives none, and there is no SIGTERM
  analog at all (Ctrl-Break is the nearest deliverable identity).
- **OS limitation**: Windows has no signal mechanism; console control
  events are the only asynchronous termination-adjacent notifications
  the OS delivers to user code.
- **Pinning tests**: `linux_signal_source_defers_and_coalesces`
  (behavioral) and `windows_signal_source_installs` (installation-level;
  the test documents why delivery is not asserted on headless CI).
- **Accepted**: 2026-07-19, with the D6 extraction.

## 004 — a symlink must declare file-vs-directory at creation on Windows

- **Linux**: `Dir::symlink` creates a single kind of object (`symlinkat`);
  the link resolves to whatever `target` turns out to be — a file, a
  directory, or nothing at all — with no distinction at creation time.
- **Windows**: the NT reparse point backing a symlink must be created as
  either a file-type or a directory-type object (`FILE_NON_DIRECTORY_FILE`
  vs. `FILE_DIRECTORY_FILE` on the creating `NtCreateFile`) — there is no
  reparse tag meaning "either." This backend decides by best-effort
  `metadata`-ing `target` relative to the same `Dir` capability: an
  existing directory there makes a directory-type link; anything else (a
  file, a dangling target, an absolute target, or one elsewhere entirely)
  falls back to file-type. A dangling link later satisfied by a directory
  stays file-type on Windows until recreated — real tooling (`mklink`,
  `CreateSymbolicLinkW`) hits the exact same requirement, this is not a
  gap specific to this backend.
- **OS limitation**: `FSCTL_SET_REPARSE_POINT`'s `REPARSE_DATA_BUFFER` has
  no "resolve lazily" mode; the object type is fixed at the `NtCreateFile`
  that creates the reparse point, before the reparse data is even
  attached.
- **Downstream effect**: which removal call works also differs. A
  directory-type link is removed like a directory (`remove_dir`); a
  file-type link, like a file (`remove_file`) — mirroring how `mklink /D`
  targets need `rd`, not `del`. Linux's `remove_file` works uniformly on
  any symlink regardless of what it points at. The parity suite's own
  cleanup tries `remove_file` first, falling back to `remove_dir`, rather
  than pinning which one Windows requires.
- **Pinning tests**: the symlink-to-directory block in each backend's
  `tests/parity.rs` `assert_fs_behavior` (`dirlink`).
- **Accepted**: 2026-07-19, with the symlink slice (D11, convergence
  roadmap).

## 005 — no execute-permission bit for a regular file on Windows

- **Linux**: `Dir::access`'s `execute` bit is a real, independently
  settable permission (`faccessat`'s `X_OK`); a plain data file created
  with the default mode (`0o666`, no execute for anyone) refuses it with
  `PermissionDenied`, regardless of who owns it or what umask was in
  effect (umask only removes bits, and there were none to begin with).
- **Windows**: there is no execute-permission bit on a regular file's
  ACL for `access` to check — execute is a property of file type/
  extension (`.exe`, `.bat`, …), not an access-control entry consumer
  code inspects. `execute` is therefore granted unconditionally once the
  entry is confirmed to exist, the same behavior every practical Windows
  `access()`/`_waccess` implementation gives.
- **OS limitation**: Windows security descriptors have no ACE type
  corresponding to POSIX's execute bit; NTFS execute-ability is
  determined by the loader at execution time (PE header, extension
  associations), not by a bit `access` could query in advance.
- **Pinning tests**: `linux_access_denies_execute_on_a_plain_file` /
  `windows_access_grants_execute_unconditionally` in each backend's
  `tests/parity.rs` — deliberately dedicated, backend-only tests rather
  than a shared assertion, since the two backends' correct behaviors are
  opposites for the identical setup.
- **Accepted**: 2026-07-19, with the faccessat slice (D11, convergence
  roadmap).

## 006 — no POSIX mode-bit/ownership model on Windows

- **Linux**: `Dir::unix_mode` returns real `setuid`/`setgid`/`sticky`
  bits and the owning `uid`/`gid` (`fstatat`'s `st_mode`/`st_uid`/
  `st_gid`) — `test -u/-g/-k/-O/-G`'s donor material (D11).
- **Windows**: there is no POSIX mode-bit or uid/gid concept at all —
  NTFS security descriptors (DACLs of per-trustee access-control
  entries keyed by SID) are a wholly different ownership and permission
  model, not a superset or subset representable as mode bits.
  `Dir::unix_mode` returns `Ok(None)` rather than a fabricated
  zeroed-out `Some(UnixMode)`, which would misrepresent "not modeled"
  as "modeled and empty."
- **OS limitation**: there is no `setuid`/`setgid`/sticky-bit analog in
  an NTFS ACL, and Windows security principals are SIDs, not small
  integer uid/gid values — there is no lossless mapping either
  direction.
- **Pinning test**: `windows_unix_mode_is_always_none` in
  `platform-windows/tests/parity.rs`; the mock's own
  `unix_mode_is_a_deterministic_default_not_none` pins the opposite
  choice mock makes (a real `Some`, deliberately not mirroring the
  Windows `None` — the mock still has no permission model, but "not
  modeled" isn't the same claim as "this OS has no such concept").
- **Not a divergence**: `Dir::file_id` (`test -ef`'s donor material) —
  both backends answer this one identically in contract (equality means
  same underlying file), even though the wire representation differs
  ((dev, ino) via `fstatat` vs. (volume serial, file index) via
  `GetFileInformationByHandle`); `FileId` is opaque specifically so that
  difference never surfaces to a consumer.
- **Accepted**: 2026-07-19, with the faccessat slice's sibling
  (`test`-predicates donor material, D11, convergence roadmap).

## 007 — no mode-bit narrowing on a Windows `AF_UNIX` bind

- **Linux**: `Net::unix_listen` narrows the freshly bound socket file to
  `0600` (owner read/write only) via `chmod`, right after `bind` — the
  mode-0600 half of D16's agreed shape (rusty_tail's LocalAPI, shh's
  agent socket), since a bare `bind` otherwise leaves the file at
  whatever the process umask allows.
- **Windows**: Winsock's `AF_UNIX` bind has no POSIX-chmod equivalent to
  narrow the bound file with — the same underlying gap `unix_mode`
  (#006) already registers, applied here to a socket file instead of an
  arbitrary one. `unix_listen` still succeeds; the bound file is left at
  the filesystem's own ACL defaults instead of forced to owner-only.
- **OS limitation**: identical to #006's — no POSIX mode-bit model on
  Windows at all, so there is nothing for `chmod`'s narrowing step to
  target.
- **Not a divergence**: the stale-cleanup-bind half of the same D16
  shape — both backends implement it identically (a throwaway probe
  connect distinguishes a stale leftover file from a live listener's
  path; see `docs/behavior/net.md`'s Unix domain sockets section). Only
  the mode-narrowing half has a real cross-backend gap.
- **Accepted**: 2026-07-20, with the Unix sockets slice (D16, convergence
  roadmap Phase 5).

## 008 — no general signal delivery or numeric process-group join on Windows

- **Linux**: `Child::kill_tree`/`kill_single` deliver any portable
  `Signal` (`Term`/`Int`/`Hup`/`Quit`/`Kill`/`Stop`/`Cont`) via `kill`/
  `killpg`; `GroupSpec::JoinGroup(pgid)` places a spawned child straight
  into an existing process group via `POSIX_SPAWN_SETPGROUP` with that
  pgid, the same race-free at-spawn placement `NewGroup` already uses.
- **Windows**: `kill_tree`/`kill_single` accept only `Signal::Kill`
  (`TerminateJobObject`/`TerminateProcess`, unchanged from this trait's
  pre-`Signal` behavior); every other `Signal` variant is `Unsupported`.
  `GroupSpec::JoinGroup` is `Unsupported` at `spawn` — refused before
  spawning anything, not silently downgraded to `Inherit`/`NewGroup`.
- **OS limitation**: Windows has no general signal-delivery mechanism —
  `TerminateProcess`/`TerminateJobObject` (unconditional termination)
  and `GenerateConsoleCtrlEvent` (console control events, restricted to
  processes sharing the sender's console — already the divergence-003
  identity set) are the only asynchronous notifications the OS can send
  to an arbitrary already-running process; there is no `SIGSTOP`/
  `SIGCONT`/`SIGTERM`/`SIGQUIT` analog to route the other `Signal`
  variants to. Separately, Windows process groups are Job Object
  *handles*, not the small integer pgids POSIX process groups are —
  there is no "start this child already inside numeric group N"
  primitive for `JoinGroup` to call.
- **Pinning tests**: `windows_kill_signal_is_kill_only` /
  `windows_join_group_is_unsupported` /
  `windows_wait_job_is_unsupported` in
  `platform-windows/tests/parity.rs`; the Linux-side positive behavior
  is pinned by `linux_kill_signal_is_portable` /
  `linux_process_group_join` /
  `linux_wait_job_observes_stop_and_continue` in
  `platform-linux/tests/parity.rs`.
- **Accepted**: 2026-07-21, with the `kill_cmd`/`fg_cmd`/`bg_cmd`
  forcing-consumer slice (rustils#44/#46 — `nexus-rush/src/job.rs` via
  `baileyrd/nexus#454`).

## 009 — no `chmod`-equivalent write path on Windows

- **Linux**: `Dir::set_unix_mode` sets the `setuid`/`setgid`/`sticky`
  special bits and the standard `rwx` permission bits at `rel` via
  `fchmodat(dirfd, rel, mode, 0)` — the write-side counterpart to
  `unix_mode` (#006). Follows a terminal symlink (no
  `AT_SYMLINK_NOFOLLOW`): the kernel does not implement changing a
  symlink's own permissions at all, so this changes the target's mode,
  matching `chmod(1)`.
- **Windows**: `Dir::set_unix_mode` is `Err(ErrorKind::Unsupported)`,
  unconditionally — never a silent `Ok(())`. Unlike #007's
  best-effort `unix_listen` mode-narrowing step (where the overall
  operation still succeeds without it), a `set_unix_mode` call is the
  caller's entire, explicit ask; silently doing nothing would
  misrepresent success — the same reasoning behind #008's
  `Signal`/`GroupSpec::JoinGroup` refusals.
- **OS limitation**: identical to #006's — no POSIX mode-bit model in
  an NTFS ACL, and no lossless mapping for `setuid`/`setgid`/sticky
  either direction. Same underlying gap as #006, applied to the write
  side instead of the read side.
- **Pinning tests**: `windows_set_unix_mode_is_unsupported` in
  `platform-windows/tests/parity.rs`; the Linux-side positive behavior
  is pinned by `linux_chmod_sets_real_permission_and_special_bits` in
  `platform-linux/tests/parity.rs`, checked against a raw `libc::stat`
  call issued directly by the test (same discipline as #006's sibling
  `Metadata`/`UnixMode` pinning test). `platform-mock`'s own
  `set_unix_mode_succeeds_but_does_not_change_the_deterministic_default`
  (`platform-mock/src/fs.rs`) pins the mock's own no-op-success choice,
  which is *not* this divergence (mock still has no permission model at
  all, per #006 — it isn't claiming "no OS concept" the way Windows is).
- **Not a divergence, but a related gap**: under the `track-p` feature,
  `Dir::set_unix_mode` is also `Unsupported` on Linux — `rusty_libc` has
  no `chmod`/`fchmodat` binding yet at the pinned rev. This is a
  temporary Track-P completeness gap, not an OS limitation (`chmod(2)`
  exists and works fine on Linux under Track P's own target kernel), so
  per this document's own rule ("cites only an OS limitation, never
  implementation convenience") it does not get a numbered entry here —
  see `docs/behavior/fs.md` and `sys/fdio.rs::set_unix_mode`'s track-p
  comment instead. Pinned by
  `linux_chmod_is_unsupported_under_track_p`.
- **Accepted**: 2026-07-23, with coreutils gap backlog #64's write-side
  half (`unix_mode`'s read side landed 2026-07-21 as part of #63/#65).

## 010 — no adopting an externally-spawned pid on Unix

- **Windows**: `Spawner::adopt(pid)` opens an already-running process by
  pid (`OpenProcess`) and places it into a fresh kill-on-close Job
  Object (`AssignProcessToJobObject`) — the same mechanism
  `GroupSpec::NewGroup` uses at spawn time, applied after the fact to a
  pid this backend didn't create (e.g.
  `portable-pty::Child::process_id()`, rustils#47's forcing case:
  `nexus-terminal`'s PTY sessions are spawned by `portable-pty`, not
  through this crate). Returns a `GroupHandle`
  (`kill_tree`/`kill_single`).
- **Unix**: `Spawner::adopt` is `Err(ErrorKind::Unsupported)`,
  unconditionally.
- **OS limitation**: POSIX `setpgid(pid, pgid)` can retarget a process's
  group only when the target is both the calling process's own child
  *and* has not yet called `execve` (`EACCES` if it's a child that
  already exec'd; `EPERM` if it isn't the caller's child at all). By the
  time any caller has a pid to adopt — obtained from a third-party
  library after that library's own spawn call has already returned —
  the target has, in every realistic case, already exec'd. There is no
  POSIX primitive that places an arbitrary already-running, already-
  exec'd process into a new or existing process group the way Windows's
  handle-based `AssignProcessToJobObject` can; unlike #008's `JoinGroup`
  gap (a *spawn-time* placement Windows lacks a numeric-pgid analog
  for), this is the mirror-image limitation — a capability Unix lacks
  that Windows has, not attempted speculatively (a `setpgid` that
  sometimes works depending on exec timing would be worse than an
  honest refusal).
- **Pinning tests**: `windows_adopt_places_a_real_pid_into_a_new_job` /
  `windows_adopt_of_a_dead_pid_fails` in
  `platform-windows/tests/parity.rs`; `linux_adopt_is_unsupported` in
  `platform-linux/tests/parity.rs` (spawns a real child and adopts its
  real pid, so the refusal is provably about the operation, not a bogus
  pid). `platform-mock`'s own `adopt_succeeds_and_logs_the_pid`
  (`platform-mock/src/process.rs`) pins the mock's unconditional-success
  choice, which is *not* this divergence — the mock has no OS process
  behind an adopted pid to fail against at all, the same "no OS
  limitation to model" stance `MockChild::kill_single` already takes
  for every `Signal`.
- **Accepted**: 2026-07-23, with rustils#47.

## 011 — pty master handle shape: one pollable fd on Linux, two non-pollable handles on Windows

- **Linux**: `LinuxPtyMaster` wraps a single fd (from `posix_openpt`) that
  is both read- and write-capable and genuinely pollable — the same
  fd handles input and output, and it supports the ordinary readiness
  mechanisms (`poll`/`epoll`) any other fd does. `AsFd`/`AsRawFd` on the
  concrete type expose it directly (rustils#41/#42's Net/Tun precedent).
- **Windows**: ConPTY's master side is a *pair* of anonymous pipes — a
  write-only input pipe and a read-only output pipe, never one
  descriptor. `WindowsPtyMaster` holds both and exposes them as two
  named accessors (`input_handle`/`output_handle`) rather than a single
  `AsHandle`/`AsRawHandle` impl, since there is no single handle to
  offer honestly. Neither handle is pollable the way a socket handle is
  — anonymous pipes created by `CreatePipe` don't support
  `WaitForMultipleObjects`-style readiness signaling or overlapped I/O;
  a consumer wanting non-blocking behavior would need to bridge onto its
  own reactor with a dedicated thread, not something this trait attempts
  on a consumer's behalf.
- **OS limitation**: Unix's pty abstraction (`/dev/ptmx` + devpts) is
  built on top of ordinary, pollable fds by construction — a pty master
  fd is not a special case for readiness purposes. ConPTY, by contrast,
  is implemented as a pair of anonymous pipes wired to an internal
  conhost process; anonymous pipes on Windows have never supported
  asynchronous/overlapped I/O (unlike named pipes), so there is no
  Windows equivalent of "the master fd is pollable" to offer, regardless
  of how this crate's own trait is shaped.
- **Related, not itself a divergence**: `ClosePseudoConsole` blocking
  until conhost's internal writer thread finishes (which can deadlock
  against an un-drained output pipe) has no Linux analog to diverge
  from — closing a Linux pty master fd is an ordinary `close(2)`, no
  blocking wait involved. `sys::pty::close`'s bounded `PeekNamedPipe`
  drain before `ClosePseudoConsole` is a Windows-only teardown detail,
  not a portable-contract difference a consumer needs to know about.
- **Pinning tests**: `crates/platform-windows/tests/pty.rs`'s
  `dropping_an_undrained_master_does_not_deadlock` — CI-only (this
  crate's whole backend is cross-compile-checked from a Linux host, per
  `platform-windows/src/lib.rs`'s own module doc; nothing here executes
  outside CI's `windows-latest` leg).
- **Accepted**: 2026-07-23, with rustils#83.

## 012 — console acquisition exists only on Windows

- **Windows**: `platform::term::ConsoleAcquisition` —
  `alloc_console`/`attach_console`/`free_console` — lets a process that
  starts with no console (the GUI-subsystem, `/SUBSYSTEM:WINDOWS`,
  default) acquire one on demand, implemented by `WindowsTerminal`.
- **Linux**: no implementor at all — `ConsoleAcquisition` is not
  implemented by `LinuxTerminal`, not `Err(ErrorKind::Unsupported)` at
  runtime. A caller that needs this capability opts in via the trait
  bound at compile time, the same shape `JobControlTerminal` already
  established in the opposite direction (Unix-only, no Windows
  implementor).
- **OS limitation**: the GUI-subsystem/console-subsystem split
  (`/SUBSYSTEM:WINDOWS` vs `/SUBSYSTEM:CONSOLE`, decided at link time)
  is a PE/Win32 loader concept with no Unix analog — every Unix process
  either inherits a controlling terminal at exec time or has none, with
  no separate "go acquire a console now" step to model. Offering
  `alloc_console`/`attach_console` as fallible-but-present methods on
  Linux would invite a caller to write Windows-only logic against a
  nominally portable API; leaving the capability off the trait entirely
  (rather than on it and always failing) makes that impossible instead
  of just unlikely, the same reasoning `JobControlTerminal`'s own
  divergence (this registry, D9/D1) already accepted for its own
  Windows gap.
- **Pinning tests**: `crates/platform-windows/tests/
  console_acquisition.rs` — CI-only, same discipline as entry #011
  above (this crate's backend is cross-compile-checked from a Linux
  host; nothing here executes outside CI's `windows-latest` leg). No
  Linux-side pin: there is no method to call and fail, so there is
  nothing for a Linux test to assert against — the absence of the trait
  impl itself is the whole of the divergence, verified by `cargo check`
  simply having no `impl ConsoleAcquisition for LinuxTerminal` to find.
- **Accepted**: 2026-08-05, with the console-acquisition slice
  (`docs/design-discussion-console.md`, extraction map D9).

## 013 — `open_dir`/`create_dir` reach mount-confinement (R2) on Linux, only atomic link-confinement on Windows

- **Linux**: `LinuxDir::open_dir`/`create_dir` are Rusty-Mill **R2** on a
  5.6+ kernel — `sys::fdio::openat_r2`'s raw `openat2` requests
  `RESOLVE_NO_SYMLINKS | RESOLVE_NO_XDEV` together, so resolution is
  refused both for a symlink anywhere in the path *and* for crossing a
  filesystem/mount boundary, atomically, in one kernel call.
- **Windows**: `WindowsDir::open_dir`/`create_dir` reach only the
  link-confinement half — `sys::nt::open_relative_r2`'s `OBJ_DONT_REPARSE`
  rejects a reparse point anywhere in resolution, the `RESOLVE_NO_SYMLINKS`
  equivalent, but `NtCreateFile`'s `OBJECT_ATTRIBUTES` has no admitted
  flag this crate found meaning "refuse to cross a volume boundary while
  resolving this path" the way `RESOLVE_NO_XDEV` does.
- **OS limitation**: `openat2`'s `resolve` bitmask was purpose-designed
  (Linux 5.6, 2020) as one atomic containment primitive covering both
  link- and mount-confinement together; NT's object-manager-era
  `OBJECT_ATTRIBUTES.Attributes` flags predate that design by decades and
  were never extended with a symmetric mount-boundary bit — a caller that
  wants Windows mount-confinement has to detect the crossing itself
  (e.g. comparing volume serial numbers before and after resolution),
  which is a stat-then-open race, not an atomic OS guarantee, and out of
  scope for this slice (`docs/behavior/fs.md`'s own honesty rule: don't
  claim a guarantee this crate can't back with a real atomic mechanism).
- **Pinning tests**:
  `linux_open_dir_rejects_a_symlink_in_an_intermediate_component` /
  `linux_create_dir_rejects_a_symlink_in_an_intermediate_component`
  (`crates/platform-linux/tests/parity.rs`) pin the link-confinement half
  both backends share. Linux's mount-confinement half is pinned by
  `linux_open_dir_rejects_a_mount_crossing_in_an_intermediate_component` /
  `linux_create_dir_rejects_a_mount_crossing_in_an_intermediate_component`
  (`crates/platform-linux/tests/mount_crossing.rs`) — mounts a real
  `tmpfs` and asserts `RESOLVE_NO_XDEV` rejects resolving across it with
  `ErrorKind::CrossesDevices`, with an R1-op sanity check (`metadata`
  still resolves across the same boundary) proving the rejection is R2's
  deliberate containment gain, not generic breakage. `mount(2)` needs
  `CAP_SYS_ADMIN`, the same gap `tests/tun_parity.rs`'s `tun_or_skip!`
  already practices honesty for: the suite skips gracefully on an
  unprivileged host and runs for real in CI's privileged job
  (`.github/workflows/ci.yml`) — see
  `crates/platform-linux/src/fs.rs`'s module doc for the precise
  per-backend R-level claim this entry backs.
- **Accepted**: 2026-08-10, with the Rusty-Mill fs R2/D2 slice. Mount-
  crossing containment test-verified 2026-08-10, closing the gap the
  Rusty-Mill `TRIAL-0002` comparison record (`RT-002`) disclosed.

## 014 — directory durability after `write_atomic`'s rename: D2 on Linux, D1 on Windows

- **Linux**: `Dir::write_atomic`'s inherited default now calls
  `Dir::sync_dir` after its publishing rename; `LinuxDir::sync_dir` calls
  `fsync` on the capability's own `O_DIRECTORY` fd, reaching Rusty-Mill
  **D2** ("namespace synchronized") — the rename's own directory-entry
  mutation is durable, not just the renamed file's content.
- **Windows**: `WindowsDir` has no `sync_dir` override — the trait's
  default no-op stands, so `write_atomic` stays **D1** ("content
  synchronized") on this backend.
- **OS limitation**: `fsync(2)` on a directory fd is Linux's own
  documented, supported mechanism (`fsync(2)`'s man page states it
  explicitly) for exactly this durability question. Windows's nearest
  candidate, `FlushFileBuffers` on a directory handle opened with
  `FILE_FLAG_BACKUP_SEMANTICS`, is not similarly documented by Microsoft
  for directory handles specifically, and has a known history of
  surprising, driver-dependent behavior for non-regular-file handles —
  there is no way to back a D2 claim here with verified evidence rather
  than a syntax check that happens to compile. This stays D1 pending a
  future slice with a real Windows/NTFS test rig to establish the actual
  behavior live, per this backend's own `Dir for WindowsDir` doc comment.
- **Pinning tests**:
  `write_atomic_fsyncs_the_directory_after_the_publishing_rename`
  (`crates/platform-linux/tests/parity.rs`) — strace-verified: the
  directory `fsync` fires strictly after the publishing `renameat2`. No
  Windows-side pin exists or is claimed, matching the honest D1 posture
  above; nothing to prove for a call this backend deliberately does not
  make.
- **Accepted**: 2026-08-10, with the Rusty-Mill fs R2/D2 slice.
