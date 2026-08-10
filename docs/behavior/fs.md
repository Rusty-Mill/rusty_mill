# Behavior Spec — fs (Dir / File)

**Capability identity & maturity** (#114): `rustils.fs.dir` — Stable;
`rustils.fs.file` — Stable; `rustils.fs.anonymous_file` (`memfd_create`) —
Trial (single Linux backend, no Windows/mock counterpart yet). See
`docs/rfc-v2.md` §10 for the numbered decision log these capabilities
trace back to (D-6, D-11).

The parity suite (`crates/platform-linux/tests/parity.rs` today; extracted
to a shared crate when a third backend lands) asserts this spec against
every backend. A backend that cannot honor a line gets a numbered entry in
`../divergences.md` citing the OS limitation — never implementation
convenience.

## Specified

- `open` with `create_new` on an existing entry fails `AlreadyExists`.
- `open` with `truncate` on an existing file leaves it zero-length.
- Reads and writes are byte-faithful: no encoding validation or conversion
  anywhere in the stack; names are `OsStr` and may be non-UTF-8 on unix.
- `metadata` on a missing entry fails `NotFound` and carries the path.
- `remove_dir` on a non-empty directory fails `DirectoryNotEmpty`.
- `remove_file` on a directory fails `IsADirectory`; `remove_dir` on a file
  fails `NotADirectory`.
- Child capabilities from `open_dir` observe the live tree (handle
  semantics), not a snapshot.
- `sync_all` blocks until a file's writes are durable (`fsync` /
  `FlushFileBuffers`) — distinct from `flush`, which has nothing to do
  on either backend since a synchronous `write` has no userspace buffer.
- `rename`/`rename_no_replace` operate on two names **within the same
  directory capability** (both relative to `self`) — cross-directory
  rename is not exposed; a consumer needing it opens the common
  ancestor. `rename` replaces an existing `to` atomically (concurrent
  readers never observe `to` absent); `rename_no_replace` refuses with
  `AlreadyExists` instead, and the check-and-rename is atomic in the
  kernel — no consumer-visible TOCTOU window (D11, convergence roadmap
  Phase 3).
- `write_atomic` (default-provided on `Dir`, RFC v2 §5.3): durably
  publishes `contents` at `rel`, never leaving a partially-written or
  missing file observable at that name, even across a crash between the
  write and the publishing rename. Sequence: write to a same-directory
  temp name → `sync_all` the temp file (durability *before* publish,
  not after) → close → `rename` over `rel` → `sync_dir` (below); the
  temp file is best-effort removed if the write/sync step fails.
  Composed entirely from `open`/`write`/`sync_all`/`rename`/`sync_dir` —
  one implementation, shared by every backend including future ones.
- `sync_dir` (`Dir`, Rusty-Mill D2 slice): directory-namespace
  durability, distinct from `File::sync_all`'s content durability —
  blocks until the containing directory's *own* mutation (a rename, a
  create, a remove — the directory entry itself, not any file's bytes)
  is acknowledged by stable storage. Default-provided as a no-op
  `Ok(())` so a backend with nothing meaningful to offer isn't forced to
  fabricate one; `write_atomic` calls it uniformly after its publishing
  rename regardless. **Linux**: `LinuxDir::sync_dir` calls `fsync` on
  the capability's own `O_DIRECTORY` fd (valid and meaningful per
  `fsync(2)`'s own text) — `write_atomic` on this backend is Rusty-Mill
  **D2** ("namespace synchronized"). **Windows**: no override — `write_atomic`
  stays **D1** ("content synchronized") on this backend, since
  `FlushFileBuffers`'s effect on a directory handle specifically has no
  documented, verifiable guarantee this crate could respectfully claim
  (`docs/divergences.md` #014 has the full reasoning; fabricating D2
  here would be a wrong claim, worse than the honest partial gap).
- **Live-verified** (strace): `write_atomic` on Linux fires `fsync`
  strictly *before* the publishing `renameat2`, and `rename_no_replace`
  carries `RENAME_NOREPLACE` — both a one-time development-session
  trace, unlike the directory-`fsync`-after-`renameat2` ordering claim
  just above, which is pinned as a committed, re-runnable test
  (`write_atomic_fsyncs_the_directory_after_the_publishing_rename`,
  `crates/platform-linux/tests/parity.rs` — the test binary re-executes
  itself under `strace` and parses the resulting syscall log).
- **Resolution safety** (Rusty-Mill R-scale; `docs/divergences.md` #013
  has the full per-backend reasoning): `open`/`metadata`/`access`/
  `unix_mode`/`file_id`/`set_unix_mode`/`read_link`/`rename`/
  `rename_no_replace`/`remove_file`/`remove_dir`/`read_dir` are **R1**
  on both backends — anchored to the capability (`openat`'s dirfd /
  `NtCreateFile`'s `RootDirectory`), but an intermediate symlink/reparse
  point is followed transparently, and so is `open`'s terminal one (the
  "open follows symlinks transparently" line above, unchanged and still
  true on both backends — `open_dir`/`create_dir`, not `open`, is where
  R2 landed). `open_dir`/`create_dir` are **R2 on Linux** (a 5.6+
  kernel: `openat2` with `RESOLVE_NO_SYMLINKS | RESOLVE_NO_XDEV`,
  rejecting a symlink or a mount-boundary crossing anywhere in
  resolution, atomically; `ErrorKind::FilesystemLoop`/`CrossesDevices`
  respectively) or R1 unchanged (pre-5.6 kernel — `openat2`'s own
  `ENOSYS` falls back transparently, never a hard failure) and
  **R2-equivalent on Windows** (`OBJ_DONT_REPARSE` gives the same
  link-confinement atomically, unconditionally — no kernel-version gate
  needed — but there is no admitted NT flag for mount-boundary
  confinement, so this is link-confinement only, not full R2; see
  divergence #013). Neither backend claims R3: no `RESOLVE_IN_ROOT`/
  root-constraint request on Linux, no NT equivalent attempted on
  Windows — this `Dir` contract does not promise beneath-confinement
  (a relative `rel` containing `..`, or a symlink whose target is
  absolute, is not prevented from resolving outside the capability's own
  subtree today), so an R3 claim would overstate what's enforced.
- `symlink`/`read_link` (`symlinkat`/`readlinkat`, D11's symlink slice):
  `symlink` creates `link_name` as a link storing `target` **verbatim**
  — `target` is opaque content, not validated, resolved, or required to
  exist, and refuses `AlreadyExists` if `link_name` already names an
  entry. `read_link` returns exactly the bytes `symlink` was given, an
  exact round trip regardless of whether `target` is relative or
  absolute. `metadata` on a symlink is lstat-style (already true before
  this slice, via `AT_SYMLINK_NOFOLLOW`/`FILE_OPEN_REPARSE_POINT`): it
  classifies the link itself as `FileType::Symlink` without following
  it, even when the target is dangling. `open` (unchanged) follows
  symlinks transparently, the same as it always has.
- Windows requires declaring file-vs-directory at symlink-creation time,
  with a consumer-visible effect on which removal call applies —
  `docs/divergences.md` #004, not asserted as uniform behavior here.
- `access` (`faccessat(2)`, D11's faccessat slice): probes whether every
  bit set in `AccessMode` (`read`/`write`/`execute`) is permitted for
  `rel`, `Err(PermissionDenied)` if any requested bit is refused. An
  empty `mode` is a vacuous `Ok(())` — including for a name that doesn't
  exist, since existence is `metadata`'s job, not this one's; both
  backends special-case this explicitly rather than letting an
  all-`false` mode fall through to a real probe (on Linux, `mode == 0`
  is `faccessat`'s own `F_OK`, a different check than "vacuous yes"; the
  bug of *not* special-casing it was caught by the parity suite itself
  before this landed). Follows a terminal symlink, like `open` and
  unlike `metadata`. Uses **real**, not effective, uid/gid on Linux —
  the plain `faccessat` syscall's own semantics, not glibc's userspace-
  only `AT_EACCESS` emulation, kept consistent with what Track P's
  `rusty_libc::fs::faccessat` can support (no flags parameter at all).
  On Windows, `read`/`write` are answered by a trial open with the
  matching access mask, immediately closed — the actual operation this
  probe predicts, not a separate ACL query.
- Windows has no execute-permission bit at all for a regular file —
  `execute` is granted unconditionally once existence is confirmed —
  `docs/divergences.md` #005, pinned by dedicated backend-only tests
  rather than a shared assertion (the two backends' correct answers are
  opposites for the identical setup).
- `unix_mode`/`file_id` (`test`'s `-u/-g/-k/-O/-G/-ef` donor material,
  D11's faccessat-slice sibling): `unix_mode` returns real
  `setuid`/`setgid`/`sticky` bits and owning `uid`/`gid` where the OS
  has the concept at all (`Ok(None)` where it doesn't — Windows,
  `docs/divergences.md` #006 — not a fabricated zeroed-out value).
  `file_id` is an opaque, equality-only per-OS file identity (POSIX
  `(dev, ino)`; Windows `(volume serial, file index)` via
  `GetFileInformationByHandle`) every backend answers identically in
  contract: the same entry queried twice yields equal ids, two distinct
  entries yield different ones. Neither follows a terminal symlink,
  matching `metadata`.

- `Metadata::nlink`/`modified` (coreutils gap backlog #63, `ls -l`'s
  donor material): unlike `UnixMode`, both fields are portable — Linux
  (`st_nlink`/`st_mtime`) and Windows
  (`FILE_STANDARD_INFO::NumberOfLinks`/`FILE_BASIC_INFO::LastWriteTime`)
  both genuinely have a link count and a modification time, so there is
  no `Option` here. `nlink` is always at least 1; `modified` is never in
  the future relative to a write that just happened — both asserted in
  the shared parity suite. Backend-specific exactness (the value really
  matches what the kernel/NTFS reports, not just "some plausible
  number") is pinned per backend against a second, independent
  source: Linux against a raw `libc::stat` call
  (`linux_metadata_reports_a_real_nlink_mtime_and_permissions`),
  Windows against `std::fs::Metadata::modified()`/a raw
  `GetFileInformationByHandleEx(FileStandardInfo, ...)` call
  (`windows_metadata_reports_a_real_nlink_and_mtime`).
- `UnixMode::permissions` (coreutils gap backlog #65, `ls -l`'s donor
  material): the standard `rwxrwxrwx` bits (`mode & 0o777`), alongside
  the special bits this field already had.
- `Dir::set_unix_mode` (`fchmodat(2)` with no `AT_SYMLINK_NOFOLLOW`;
  coreutils gap backlog #64 — `unix_mode`'s write-side companion):
  takes a [`Mode`] (`setuid`/`setgid`/`sticky` + `permissions`, no
  `uid`/`gid` — that's `chown`'s job, not covered here) and sets those
  bits at `rel`. Unlike `unix_mode`/`metadata`, this **follows a
  terminal symlink** rather than operating lstat-style: Linux's kernel
  does not implement changing a symlink's own permissions at all
  (`fchmodat(..., AT_SYMLINK_NOFOLLOW)` fails `ENOTSUP` on every real
  filesystem — symlink mode bits are unused and ignored), so this
  method changes the **target**'s mode, matching `chmod(1)`'s own
  behavior when pointed at a symlink. Windows has no POSIX mode-bit
  concept to set at all — `Err(Unsupported)`, not a silent no-op
  (`docs/divergences.md` #009); a caller here explicitly asked to
  change permissions, so pretending success would misrepresent what
  happened, unlike `unix_mode`'s `Ok(None)` answer to a *query* with
  nothing to report. `platform-mock` accepts the call (existence is
  still checked, `NotFound` on a missing entry) but does not persist
  the requested bits — consistent with `unix_mode`'s own fixed-default
  answer there, which no `set_unix_mode` call changes either way (mock
  still has no real permission model, `docs/divergences.md` #006).
  Track P (RFC v2 §2): `rusty_libc` has no `chmod`/`fchmodat` binding
  yet at the pinned rev, so the `track-p` feature answers
  `Err(Unsupported)` too, pending its own call-by-call replacement —
  not asserted in the shared parity suite (a temporary Track-P
  completeness gap, not an OS limitation, so it gets no
  `docs/divergences.md` entry per that document's own rule).
- `unix_mode`/`file_id` (`test`'s `-u/-g/-k/-O/-G/-ef` donor material,
  needing the `2>&1`/`&> file` shell-redirect shape): duplicates the
  underlying OS handle (`dup(2)`/`DuplicateHandle`) into a fresh, owned
  `File` that shares the *same open-file description* as the original —
  position included. A read or write through either handle advances the
  other's next read/write position too; this is the entire reason the
  method exists (a fresh `Dir::open` of the same path gets an
  independent position instead, and cannot substitute for it). The
  clone is not itself inheritable by a spawned child — Unix: `CLOEXEC`
  set (`F_DUPFD_CLOEXEC`); Windows: not marked inheritable
  (`DuplicateHandle` with `bInheritHandle = FALSE`) — inheritance is
  `Stdio::File`'s job at spawn time, a separate, explicit step.
  Pinned by a dedicated test in each native backend's `tests/parity.rs`
  (`linux_stdio_file_try_clone_shares_offset_for_dup_style_redirect` /
  the Windows copy), not the shared `assert_fs_behavior` — exercised
  through `Stdio::File` rather than `try_clone` in isolation, since the
  redirect-duplication shape is the actual forcing use case: two
  processes' worth of writes through clones of the same file interleave
  correctly (append, not clobber at position 0) only if the position is
  genuinely shared. `platform-mock`'s own
  `try_clone_shares_the_read_position` unit test
  (`platform-mock/src/fs.rs`) pins the same property directly, needing
  no OS fixture at all.
- `AnonymousFile::create_memfd` (D11, landed independently per
  `docs/decision-request-fork-execve.md`'s option 3 — not part of the
  larger parked fork/execve decision, just cited by it): a fresh,
  memory-backed `File` with no filesystem namespace entry at all,
  unlinked from the moment it exists. Linux: `memfd_create(2)` with
  `MFD_CLOEXEC`. `name` is a debugging label only, not a path — two
  calls with the same `name` produce two fully independent files
  (`platform-linux/src/sys/fdio.rs::memfd_tests::two_memfds_are_independent`,
  `platform-mock`'s own
  `anonymous_file_round_trips_and_is_independent_of_any_dir`). Live
  ground truth, not just a successful syscall return: Linux's own test
  suite confirms via `/proc/self/fd/<n>` that the kernel reports the
  fd as `(deleted)` immediately (`memfd_create_is_genuinely_unlinked`)
  — the same "check real OS state, not just `Ok(())`" discipline the
  PTY surface's own tests use. Deliberately **not** a `Dir` method:
  `memfd_create` takes no path and touches no directory capability at
  all, so putting it on `Dir` would mean an always-unused `&self`
  receiver on every call — a separate, small trait instead, mirroring
  `platform::security::Csprng`'s own shape.

## Async path (rusty_foundation_akb §9)

**Decision: sync-only, justified — no async path today, and no raw-handle
escape hatch either (unlike `net`).** Every `File`/`Dir` operation
(`read`, `write`, `sync_all`, `open`, `metadata`, `read_dir`, ...) is
ordinary blocking I/O with no non-blocking mode, no readiness-based
escape hatch, and no owned-buffer async variant.

**Why, not just "not done yet":** the closest sibling project already hit
the actual blocker. `rusty_tokio`'s io-uring file-I/O layer
(`io/uring_fs.rs`) is a *separate* owned-buffer (`IoBuf`/`IoBufMut`) type
from its borrowed-buffer `AsyncRead`/`AsyncWrite` file type, specifically
because "a borrowed-buffer async file type can't safely do this" under
cancellation — an in-flight io_uring read holds a raw pointer into the
caller's buffer, and if the calling future is dropped (cancelled) before
completion, that buffer can be freed or reused while the kernel still
writes into it. Retrofitting that onto `File::read(&mut self, buf: &mut
[u8])`'s borrowed-buffer signature would be unsound; doing it right means
a distinct owned-buffer trait, not an async variant of the existing one.

That is real trait-surface design work, not a documentation gap, so it is
tracked as a follow-up rather than attempted here — see the tracking
issue this behavior spec update was filed alongside (`docs/rfc-v2.md`'s
scope-gap tracking, RM-DEV-DEP-0001-style). Until that lands, an
async-*driven* consumer performs file I/O the same way rusty_tokio's own
`uring_fs` module does today: independently of `platform::fs`, not
through it.

**Cancellation / completion ownership / backpressure:** not applicable —
there is no async path to define them for.

## Deliberately unspecified

- `read_dir` ordering. Backends differ (mock: name order as an accident of
  BTreeMap; Linux: directory order). Consumers sort — see `coreutils::ls`.
- Fine-grained NTFS ACL semantics beyond `access`'s coarse
  read/write/execute probe and `unix_mode`'s `Ok(None)` — this spec
  stops at "can I do this" and "does this OS have mode bits at all,"
  not "what does this file's full ACL say."
- uid/gid → display-name resolution (`root`, not `0`). Deliberately
  outside this trait: it's an NSS/directory-service lookup, not
  filesystem metadata — `UnixMode::uid`/`gid` already answer "what
  number does this entry's mode word say," and a consumer that wants a
  human-readable name resolves it itself
  (`platform_linux::{user_name, group_name}`, `getpwuid_r`/
  `getgrgid_r` — coreutils gap backlog #65's `ls -l` donor material,
  used by `rls -l` but not part of `platform::fs`/`Dir` at all).
