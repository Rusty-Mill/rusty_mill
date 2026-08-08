# Convergence Roadmap

**Status:** Living document, opened 2026-07-19 against `docs/architecture.md`
(RFC v2 Amendment A3) and the D1–D16 donor inventory in
`docs/extraction-map.md`. This is *execution sequencing*, not a new
decision — it does not amend the RFC. Update it the way the extraction
map records landed-notes: mark items done in place, don't delete the
history.

Two kinds of work live here, and they land in different repos:

- **Surface work** — new or extended `platform::*` traits. Lands in
  **this repo**, follows the standing PR workflow, gated by §3 (a named
  consumer must exist before work starts).
- **Convergence work** — a parallel tool swapping its hand-rolled OS
  calls for an existing PAL trait. Lands in **the tool's own repo**.
  Each one gets called out with a "lands in" line; none of these are
  authorized to start just by appearing here — each needs its own
  go-ahead when its phase comes up.

Ordering principle: cheapest-and-already-forced first, design decisions
called out rather than defaulted, nothing built before its consumer is
named. No calendar dates, per RFC §8's discipline — phases are
dependency order, not a schedule.

## Phase map

```
Phase 1  Free convergences (no new surface)         ← start here
Phase 2  Terminal slice 2 (bracketed paste, etc.)
Phase 3  Fs second wave (D11: renameat2, atomic write, memfd)
Phase 4  Track P completion (getdents64, pidfd_open upstream)
Phase 5  Net surface (D16)
Phase 6  Security surface (D15)
Phase 7  PTY surface (D13) — landed 2026-07-23 (both Linux + Windows)
Phase 8  Tun surface (D14)
Phase 9  Windowing + Registry/Config (nexus-only, converge last)
─────────────────────────────────────────────────────────────
decided  fork/execve vs posix_spawn — DECIDED 2026-08-06: stays on
         posix_spawn indefinitely (docs/decision-request-fork-execve.md,
         docs/architecture.md's "Decided" section)
parked   MSYS2/Cygwin-level Windows job control & signal parity —
         DECIDED 2026-08-06: parked indefinitely, no consumer built
         against. All four subsystems scoped to implementation depth
         (docs/design-discussion-msys-parity.md, -pgid-table.md,
         -signals.md, -stop-cont.md, -pty.md; decision recorded in
         docs/decision-request-msys-parity.md). Revisit only if a real
         consumer appears.
```

Phases are not strictly sequential gates — 3 and 4 can interleave with
2, and nothing stops Phase 5 starting before Phase 2 finishes. The
number is priority, not a blocker on later phases.

---

## Phase 1 — Free convergences (no new PAL surface needed)

The highest-value, lowest-cost work available: tools that already have
everything they need sitting in `platform` today. Zero rustils-side
work; each is a self-contained PR in the tool's own repo.

### 1a. nexus → `platform::process` (Process, already built)

**Lands in nexus.** `nexus-terminal/src/job_object.rs` (Windows Job
Objects: CreateJobObjectW/kill-on-close) and `nexus-rush/src/job.rs`
(Unix setpgid×2/tcsetpgrp/SIG_IGN) independently re-derive D1/D2 —
mechanisms already landed here as `GroupSpec::NewGroup` +
`Child::kill_tree`. Swap both for `platform::process::{Spawner,
GroupSpec, Child}`. No new surface, no design question — pure adoption.
Proves Process holds up under a demanding, unrelated consumer.

### 1b. rusty_lines → `platform::term` (Terminal, partial — slice 1 only)

**Lands in rusty_lines.** `src/term_sys.rs`'s three-backend facade
(libc/rusty_libc/rusty_win32) can swap its `is_tty`/`get_attrs`
(window_size)/raw-mode calls for `platform::term::Terminal` today. Its
`poll_readable`/`read_chunk`/echo-off/bracketed-paste/suspend-resume
calls cannot — they need Phase 2. **First slice is worth doing now
anyway**: it's the fastest real-world exercise of the Terminal trait,
and it turns rusty_lines into the concrete forcing consumer for Phase
2's remaining facets instead of a hypothetical one.

### 1c. winargv handback — prerequisite work, not yet a convergence

**Landed here 2026-07-19.** `winargv` is now its own workspace crate
(`crates/winargv`, depending only on `platform` for error types) rather
than a module inside `platform-windows`. `platform-windows` re-exports
it (`pub use winargv;`) so nothing about its own internal use changed —
`process.rs`'s `crate::winargv::build_command_line` and the existing
`tests/winargv_oracle.rs`/fuzz target still resolve unchanged. What
changed: a handback consumer (rusty_naner's `raw_arg`-quoted command
lines, or rush's own winjob.rs) can now depend on `winargv` alone —
zero windows-sys, zero Dir/Spawner/console surface — instead of pulling
in all of `platform-windows` for one quoting module. The actual
handback PRs (rusty_naner, rush) are still open, tracked as their own
convergence work, not implied by this landing.

### 1d. rusty_win32 as a dependency (Track W) — surface work, landed here

**Landed here 2026-08-07 (D-15).** `platform-windows` gained an
off-by-default `track-w` feature routing curated call families through
`rusty_win32` — a rev-pinned git dependency, one source of truth, no
vendored fork — instead of windows-sys. Structurally identical to D-12's
`rusty_libc`/`track-p` adoption on the Linux side, including the MSRV
posture (rusty_win32's floor is 1.88, above this workspace's 1.75, so the
feature is enabled only on the windows+stable CI leg) and the
call-by-call migration discipline. First family:
`sys::fileio::read`/`write`.

Filed under Phase 1 because it needed no new PAL surface and no consumer
gate: `platform::fs`'s contract is unchanged, both configurations produce
bit-identical `PlatformError`s, and the entire platform-windows suite
re-runs under `--features track-w` as the equivalence test. It is
*surface* work only in the Cargo sense — a new feature is a public
interface addition (hence the `y` bump), not a new trait.

What this is **not**: a lower tier. Track P descends below libc to the
kernel ABI; Windows publishes no supported tier below a documented DLL
export, so `track-w` swaps the binding's provenance and leaves D-1's
floor decision intact. The write-up is `docs/learning/003-…`; the
asymmetry is called out there, in D-15, and in `docs/architecture.md`
rather than left for a reader to infer from the symmetric feature name.

Remaining Track W work, in the same call-by-call spirit and each its own
slice — not authorized by this landing, listed so the migration has a
visible shape: `sys::proc`'s spawn/wait/job families (the donor's own
strongest material, D2), `sys::console`'s mode and screen-buffer calls
(D9), and `sys::handle`'s pipe/duplicate/inheritability calls (D5).
Families where rusty_win32 has no binding at the pinned rev — `sys::nt`
above all, whose `NtCreateFile` handle-relative opens are this backend's
whole capability model — stay on windows-sys in both configurations, the
same way `fchmodat` stays on libc in both Track P configurations.

**Slice 2 landed 2026-08-07 — `sys::proc`'s spawn/wait/job families.**
Nine call sites migrated: `create_kill_on_close_job` (`job::create` +
`set_kill_on_close`), `adopt` (`process::open_by_pid` + `job::assign`),
`spawn`'s assign and resume steps, `terminate_job`, `terminate_process`,
`try_wait`/`wait` (`process::wait`), and `wait_many`'s single
`WaitForMultipleObjects` call (`process::wait_any`). `sys::pty` inherits
the job-creation migration, sharing that helper already.

**Closed, not deferred: `CreateProcessW` never migrates.** This is the
one item on the Track W list that gets struck rather than scheduled, so
a later slice doesn't reopen it. `rusty_win32::process::spawn_suspended`
(a) passes a bare zeroed `STARTUPINFOW` and documents the std-slot-swap
model instead — the model D5 step 4 above records this repo deciding
*against*, and whose rejection is what makes `Stdio::{Null, Pipe, File}`
possible; (b) passes `NULL` for `lpCurrentDirectory`; (c) takes `&str`
where winargv produces `&[u16]`. These are not donor gaps to file
upstream: a `spawn_suspended` fixed on all three counts would simply be
this crate's `spawn` living in the other repo. Two correct answers to two
different consumers' questions — see `docs/learning/004-…`.

`wait_many`'s 64-handle chunking likewise stays this crate's own; the
donor's `wait_any` reports `ERROR_INVALID_PARAMETER` past the cap exactly
as the raw call does, which is right for a binding and wrong for the
§5.6 reactor.

**Slice 3 landed 2026-08-07 — `sys::console` and `sys::handle`,
completing the list.** `sys::handle`'s `duplicate` and
`OwnedWinHandle::drop`; `sys::console`'s std-handle lookup, the
`GetConsoleMode`/`SetConsoleMode` pair (factored into two-armed helpers,
so the ten public functions over it stay single-bodied), viewport query,
`poll_readable`, `read_chunk`, `alloc`/`free`/`attach`, and the std-slot
repoint. One boundary swap: the donor's `window_size` returns
`(cols, rows)` against this crate's `(rows, cols)` — same type, opposite
order, nothing in the compiler to catch it (`docs/learning/005-…`).

**Declined for now, not closed:** `reopen`'s
`CreateFileW("CONIN$"/"CONOUT$")`. The donor's `fs::open_file` differs
only by `FILE_ATTRIBUTE_NORMAL` vs `0`, very probably inert on an
`OPEN_EXISTING` console-device open — but it backs the `has_console`
probe, whose last wrong answer cost a CI debugging session, and the
claim can't be verified from this repo's Linux-host workflow. Worth
distinguishing from `CreateProcessW` above, which is closed *permanently*:
a migration list needs both verbs.

**Track W's migration list is now empty** *of the families this document
had enumerated*. See the second correction below — that list was itself
incomplete, so this sentence was never the "nothing left to migrate"
claim it reads as. What remains on windows-sys in both configurations is
either permanently closed (`CreateProcessW`), held pending better
evidence (`reopen`), has no donor binding, or — as it turns out — was
simply never enumerated here.

> **Correction, 2026-08-07.** The sentence above originally listed
> `sys::net` among the families with "no donor binding at all". That was
> wrong: rusty_win32 has had a full TCP/UDP socket surface
> (`socket`/`bind`/`listen`/`accept`/`connect`/`send`/`recv`/`sendto`/
> `recvfrom`/`local_addr`/`peer_addr`/`set_sockopt`) since its round-2
> subsystems landed. The claim was made from the shape of the *remaining
> windows-sys calls* rather than from reading the donor, and it would
> have sent the next reader looking upstream for work already done.
> `sys::security` was correctly listed — that module's donor namesake is
> the ACL/SID surface, which shares nothing with CSPRNG, Credential
> Manager or certificate stores.

### Slice 4 (2026-08-07) — bindings added upstream, security migrated

The first slice to run in the other direction: instead of picking from
what the donor had, it added what the donor lacked.

**Landed upstream** (rusty_win32): `crypto` (`BCryptGenRandom`),
`credential` (Credential Manager, `CRED_TYPE_GENERIC`), `certstore`
(system certificate stores, read-only), `net::set_nonblocking`
(`ioctlsocket(FIONBIO)`), and AF_UNIX support in `net`
(`AddressFamily::Unix`, `UnixSocketAddr`, `bind_unix`/`connect_unix`/
`accept_unix`/`local_addr_unix`).

**Landed here:** `sys::security` migrated in full — all three families
(`fill_random`, `credential_set`/`credential_get`, `load_anchors`), to
the point where the module's `windows-sys` import is itself
`cfg(not(track-w))`. Plus `sys::net::set_nonblocking`.

**Not landed here, and not blocked either:** the rest of `sys::net` —
`socket`/`bind`/`listen`/`accept`/`connect`/`send`/`recv`/`sendto`/
`recvfrom`/`local_addr`/`peer_addr`, and the AF_UNIX paths the new
upstream bindings were added for. The bindings exist and the mapping is
understood; what makes this its own slice rather than a tail on this one
is an address-conversion layer: this crate speaks `std::net::SocketAddr`
and raw `SOCKADDR_IN`/`SOCKADDR_IN6`, the donor speaks its own
`SocketAddr` enum and `UnixSocketAddr`, and every one of those calls
crosses that boundary. It is mechanical but it is not small, and doing
it half-attentively in the margin of another slice is how a transposed
tuple (see `docs/learning/005-…`) gets shipped. Next slice.

### Slice 5 (2026-08-07) — `sys::net` migrated; Track W complete

The address-conversion layer built, and every socket family behind it:
`socket`/`bind`/`listen`/`accept`/`connect`/`send`/`recv`/`sendto`/
`recvfrom`/`getsockname`/`getpeername`/`setsockopt`/`closesocket`/
`WSAStartup`, plus the AF_UNIX paths. A two-armed primitive layer keeps
the fifteen public functions single-bodied.

Two calls remain on windows-sys, for different reasons: `DeleteFileW`
(a filesystem unlink in a net module; the donor's `fs::delete_file`
takes `&str` where this has an `OsStr` — `sys::fileio`'s business, not
this module's) and `unix_peer_addr`'s `getpeername` (**a real upstream
gap**: the donor has `local_addr_unix` but no peer counterpart).

Two behaviours got *more correct*, not merely equivalent:
`unix_listen`'s stale-socket retry and `is_stale_socket` both re-read
`WSAGetLastError` after the failing call returned — a latent race, since
any intervening Winsock call overwrites the slot. Both now branch on the
`ErrorKind` carried out of the call itself.

~~**Track W is now complete across `platform-windows`.**~~ **It is not
— see the correction immediately below.** Of the families *this document
listed*, every one either migrated, is permanently closed
(`CreateProcessW`), is declined pending evidence (`reopen`), or is a
named upstream gap (`unix_peer_addr`) — plus `sys::nt`, which has no
donor bindings at all and would need an `ntdll` admission rusty_win32
has deliberately never made.

> **Correction, 2026-08-08 — Track W is *not* complete.** The claim above
> was wrong, and wrong in an instructive way: it was true of the
> migration list in §1d and false of the codebase, because the list was
> built during slice 1 from a survey of the then-known modules and never
> re-derived against the donor's actual surface. An audit prompted by
> the question "is there anything left?" found three families that were
> never on it:
>
> - **`sys::csignals`** — still calls `SetConsoleCtrlHandler` directly,
>   though `console::install_ctrl_handler`/`remove_ctrl_handler` have
>   existed since rusty_win32's *Phase 1*. The donor's flagship binding,
>   missed entirely. → rustils#107
> - **`sys::proc`'s pipe/handle helpers** — `make_pipe` and
>   `inheritable_dup_of_std` still call `CreatePipe`,
>   `SetHandleInformation`, `GetStdHandle` and `DuplicateHandle` raw. The
>   last one bypasses the `sys::handle::duplicate` wrapper slice 3
>   already migrated. → rustils#108
> - **`sys::pty`** — the whole ConPTY surface, never audited, against a
>   donor `conpty` module that covers the lifecycle, the attribute list,
>   and the pipe/read/wait primitives. → rustils#109
>
> Plus the upstream half of `unix_peer_addr`, now filed as
> baileyrd/rusty_win32#271.
>
> The lesson, and the reason this is recorded rather than quietly
> edited: **a migration list is a claim about a codebase, and it decays
> the moment it stops being re-derived from one.** Both errors this
> document has needed correcting (the `sys::net` "no donor binding"
> claim, and this one) came from reasoning about the list instead of
> re-reading the two crates. A future slice should start by enumerating
> every `w::` call reachable under `track-w` and diffing that against
> the donor's public surface — mechanically, not from this document.

---

## Phase 2 — Terminal slice 2 (D9, remaining facets)

**Landed 2026-07-19.** Forcing consumer: rusty_lines (via 1b above).
Added to `platform::term::Terminal`:

- `is_raw()` — a **live** OS-state probe (not a cached flag), so a
  consumer can notice drift from something outside its own
  `enter_raw`/`leave_raw` calls.
- `poll_readable(timeout)` / `read_chunk` — VMIN=1/VTIME=0-style
  batched reads, distinct from the multiplexed `wait_any` reactor.
  Live-verified: `rterm --raw-probe` under a real pty polls then reads
  a batched chunk.
- `set_echo(bool) -> Result<bool>` — echo toggle independent of full
  raw mode (password prompts), returning the previous state so a
  caller restores exactly.

**Scoped out on purpose, not deferred** — the other two facets in the
original plan turned out to need no new surface at all:
bracketed paste is protocol bytes over the stream `read_chunk` already
exposes (no OS call, so it stays consumer-expressible, not
PAL-owned); cooked↔raw suspend/resume is exactly a second
`leave_raw()`/`enter_raw()` pair — the existing save/restore contract
already produces the right outcome. `docs/behavior/term.md` records
this as a deliberate scoping decision, not an oversight.

Job-control terminal handoff (`tcsetpgrp` give/reclaim) landed
separately with D1's job-control slice (2026-07-21) — see this
document's own D1 entry cross-reference, `docs/extraction-map.md`.

Console *acquisition* (`AttachConsole`/`AllocConsole`/`FreeConsole` +
`CONIN$`/`CONOUT$` reopen, `platform::term::ConsoleAcquisition`) landed
2026-08-05 — the owner's explicit call to build it speculatively
(no confirmed live consumer; the `rusty_naner` convergence this entry
originally said it was waiting on still hasn't been scheduled), the
same posture PTY hosting (D13, Phase 7) was built under. See
`docs/design-discussion-console.md` for the full design pass and
`docs/extraction-map.md`'s D9 entry for the landed-slice summary.

---

## Phase 3 — Fs second wave (D11)

**Landed (first slice) 2026-07-19.** Self-contained, no design
decisions. Added to `platform::fs`:

- `File::sync_all()` — durability (`fsync`/`FlushFileBuffers`),
  finally giving `flush`'s long-standing doc comment its distinct
  explicit companion.
- `Dir::rename`/`rename_no_replace` — same-directory rename, replace
  vs. atomic-refuse-if-exists. Linux: `renameat2` (no libc wrapper at
  this MSRV on glibc x86_64, same situation `pidfd_open` was in — the
  raw-syscall arm; rusty_libc *does* have `renameat2`, so the track-p
  arm is an ordinary split, not another escape hatch). Windows:
  `FILE_RENAME_INFORMATION` via `NtSetInformationFile` with
  `RootDirectory` set to the capability's own handle — the
  handle-relative rename this backend's ambient-path-free model needs.
  Not the Win32 `SetFileInformationByHandle`, the first thing tried: a
  live windows-latest CI run proved that wrapper rejects a non-null
  `RootDirectory` for the classic `FileRenameInfo` class with
  `ERROR_INVALID_PARAMETER` — handle-relative rename turns out to be a
  Win32-layer restriction, not an NT one (second ntdll admission,
  `ffi::nt_surface`).
- `Dir::write_atomic` (default-provided, composes `open`+`write`+
  `sync_all`+`rename` — one implementation for every backend) — the
  headline deliverable, forced by two independent donors (nexus
  `storage/atomic.rs`, rusty_naner's staged install). Strace-verified
  on Linux: `fsync` fires strictly before the publishing `renameat2`.

**Landed (symlink slice) 2026-07-19.** The item this phase originally
deferred. Added to `platform::fs`:

- `Dir::symlink`/`read_link` (`symlinkat`/`readlinkat`) — the target is
  stored verbatim, not validated or resolved; `read_link` round-trips
  the exact bytes. Linux: ordinary `symlinkat`/`readlinkat` libc
  wrappers (unlike `renameat2`, no raw-syscall escape hatch needed).
  Windows: `FSCTL_SET_REPARSE_POINT`/`FSCTL_GET_REPARSE_POINT` over a
  hand-built `REPARSE_DATA_BUFFER` (third ntdll-adjacent admission,
  `ffi::nt_surface` — a struct, not a function this time), using the
  same `addr_of!`-derived-offset technique `rename`'s
  `FILE_RENAME_INFORMATION` construction established. The one thing
  Windows requires that POSIX doesn't — declaring file-vs-directory at
  creation — is a registered divergence (`docs/divergences.md` #004),
  not papered over.

**Landed (faccessat slice) 2026-07-19.** The design pass this phase's
own predecessor said it needed. Added to `platform::fs`:

- `Dir::access` (`faccessat(2)`) — probes `read`/`write`/`execute`
  (`AccessMode`), `Err(PermissionDenied)` if any requested bit is
  refused; an empty mode is a vacuous yes, existence being `metadata`'s
  job. Linux: the plain `faccessat` wrapper with real, not effective,
  uid/gid — deliberately not glibc's `AT_EACCESS` emulation, since
  Track P's `rusty_libc::fs::faccessat` has no flags parameter at all
  and this keeps both configurations answering the identical question.
  Windows: a trial open with the matching access mask, immediately
  closed — the actual operation the probe predicts. The cross-platform
  permission-predicate question the design pass needed to answer:
  Windows has no execute-permission bit on a regular file at all, so
  `execute` is granted unconditionally once existence is confirmed — a
  registered divergence (`docs/divergences.md` #005), pinned by
  dedicated backend-only tests (the two backends' correct answers are
  opposites for the identical setup) rather than a shared assertion.

**Landed (test-predicates slice) 2026-07-19.** `test`'s `-u/-g/-k/-O/-G`
(mode bits, ownership) and `-ef` (same-file identity) — `-x` was already
`Dir::access(rel, AccessMode::execute())`, no new surface needed. Added
to `platform::fs`:

- `Dir::unix_mode` — `setuid`/`setgid`/`sticky` and owning `uid`/`gid`
  where the OS has the concept (real `fstatat` data on Linux); `Ok(None)`
  — not a fabricated zeroed-out `Some` — where it doesn't (Windows has
  no POSIX mode-bit/uid-gid model at all, a registered divergence,
  `docs/divergences.md` #006).
- `Dir::file_id` — an opaque, equality-only per-OS file identity (POSIX
  `(dev, ino)`; Windows `(volume serial, file index)` via
  `GetFileInformationByHandle`) both backends answer in the same
  contract, unlike `unix_mode`: this one has no Windows gap.

**PATH-resolution unification, corrected scope**: this donor item
turned out to already be done — `Spawner::resolve` (mechanism-level
PATH+exec-bit on Linux, PATH+PATHEXT on Windows) has existed in
`platform::process` since the process-surface work landed, and
`WindowsSpawner::spawn` already calls it internally. What remains is
entirely ecosystem-side: rush swapping its own three duplicated
PATH-walking implementations (`command -v`/`type`/completion) over to
call this already-existing API — out of scope here without `rush`'s
actual code in hand to unify against.

**Landed (`Metadata`/`UnixMode` third wave: nlink, mtime, permissions)
2026-07-21** — coreutils gap backlog #63/#64/#65's `ls -l` donor
material, forced by this repo's own reference consumer
(`coreutils::ls -l`, a real forcing use even though it's internal —
RFC v2 §3 requires a named consumer, not an *external* one).
`Metadata` gains `nlink: u64`/`modified: SystemTime` (portable —
both backends genuinely have a link count and mtime, no `Option`
needed, unlike `UnixMode`); `UnixMode` gains `permissions: u16` (the
standard `rwxrwxrwx` bits — `Dir::unix_mode`'s write-side companion,
a `chmod`-equivalent, landed 2026-07-23, see below). Linux: `statat`/`unix_mode` extended
in place (`fstatat`'s `st_nlink`/`st_mtime`/`st_mode & 0o777`, and the
Track P `statx` arm's equivalent fields) rather than adding a second
syscall. Windows: `FILE_BASIC_INFO::LastWriteTime` (already fetched
for `metadata_by_handle`, just not previously extracted) and a new
`file_standard_info` call (refactored from the private `end_of_file`
helper, which already queried `FILE_STANDARD_INFO` but discarded its
`NumberOfLinks` field). Live-verified per backend against a second,
independent source — Linux via a raw `libc::stat` call, Windows via
`std::fs::Metadata::modified()` plus a raw
`GetFileInformationByHandleEx(FileStandardInfo, ...)` call (no stable
`std` accessor exists for Windows link count —
`MetadataExt::number_of_links` is nightly-only). See
`docs/behavior/fs.md` for the full contract.

**Landed (`Dir::set_unix_mode`, `unix_mode`'s write-side companion)
2026-07-23** — coreutils gap backlog #64's remaining half, closing it
out. A new `Mode` struct (`setuid`/`setgid`/`sticky`/`permissions`,
deliberately no `uid`/`gid` — `chown`'s job, not this method's) is the
input; Linux implements it via `fchmodat(dirfd, rel, mode, 0)` (no
`AT_SYMLINK_NOFOLLOW` — the kernel has no symlink-permission concept to
target, so this follows the terminal symlink like `chmod(1)` does,
unlike `unix_mode`/`metadata`'s lstat-style contract); Windows is
`Err(Unsupported)` (`docs/divergences.md` #009, the write-side sibling
of #006), never a silent no-op, since the caller's entire ask was to
change permissions. `platform-mock` accepts the call (still enforcing
`NotFound` on a missing entry) without persisting anything, matching
`unix_mode`'s own fixed-default stance there. Landed ahead of a named
`coreutils` consumer (no `rchmod` exists) — the read side already
forced `UnixMode`'s shape onto every backend, and a permission-bit
field with no way to set it back was an increasingly conspicuous
half-finished capability in its own right. Track P: `rusty_libc` has
no `chmod`/`fchmodat` primitive at the pinned rev yet, so the
`track-p` feature also answers `Unsupported` for now — a Track-P
completeness gap, not an OS limitation, pending its own call-by-call
replacement per RFC v2 §2. See `docs/behavior/fs.md` and
`docs/coreutils-gap-backlog.md`'s Gap 3 resolution note for the full
contract.

Also added, deliberately **outside** `platform::fs` (uid/gid → display
name is a directory-service lookup, not filesystem metadata):
`platform_linux::{user_name, group_name}` via `getpwuid_r`/
`getgrgid_r` (reentrant, not the classic shared-static-buffer
variants), backing `coreutils::native::user_name`/`group_name` —
`rls -l`'s only OS-touching addition beyond `platform::fs` itself.
`coreutils::ls::{ls_long, format_long}` render the actual `-l`
long-format line, diffed **byte-for-byte identical** (aside from the
`total N` block-count header, deliberately not replicated — no
allocated-block-count field exists anywhere in `Metadata`) against
real `ls -l` in this session's own sandboxed environment across
several stress cases: wide size-column alignment, `setuid` rendering,
a zero-permission directory, a dangling symlink, and a real `nlink: 3`
from an actual subdirectory.

**Landed (`AnonymousFile::create_memfd`) 2026-08-06** — D11's
`memfd_create`, previously cited only as D4's fork-vs-malloc-lock
rationale and never surfaced as an API, landed on its own: option 3 of
`docs/decision-request-fork-execve.md`, independent of that document's
still-unresolved larger question. Linux: plain `libc::memfd_create` +
`MFD_CLOEXEC` — no `track-p` gate, `memfd_create` has had a real libc
wrapper long before this workspace's MSRV floor. Windows:
`Unsupported`, no numbered divergence — `CreateFileMappingW`'s
anonymous mapping is a fixed-size shared-memory mapping, not a
growable byte-stream `File`, so this is a missing-donor-shape gap
(`WindowsTun`'s own precedent), not an OS limitation. Live-verified,
not just a successful-syscall assertion: `sys::fdio`'s own tests
confirm via `/proc/self/fd/<n>` that the kernel reports the fd
`(deleted)` immediately. See `docs/behavior/fs.md` and
`docs/extraction-map.md`'s D11 entry for the full contract.

## Phase 4 — Track P completion

**Landed 2026-07-19, both parts.**

- **rusty_libc** ([PR #19](https://github.com/baileyrd/rusty_libc/pull/19)):
  `fs::getdents64`/`dirents` (a caller-owned-buffer syscall wrapper plus a
  zero-allocation parsing iterator over the kernel's own `d_reclen`
  chain) and `process::pidfd_open` + `wait::P_PIDFD` (the `waitid`
  pairing needed to actually use it), verified end to end there by a
  real fork + `pidfd_open` + `poll` + `waitid(P_PIDFD, ...)` test.
- **Here**: the rev pin bumped to that merge; `platform-linux::sys::fdio::read_dir`
  now has a `track-p` arm calling `getdents64`/`dirents` directly
  (`fdopendir`/`readdir`'s `DIR*` stream is glibc userspace buffering
  with no raw-syscall equivalent of its own — bypassed entirely rather
  than reimplemented), and `sys::spawn::poll_pids`'s pidfd-opening step
  is now a track-p-split `pidfd_open` helper — the raw `c::syscall`
  escape hatch is gone under `track-p` (still there, unavoidably, in the
  non-track-p arm: no libc wrapper for this syscall exists at this
  workspace's MSRV). Live-verified via strace under `--features
  track-p`: a real `read_dir` fires `getdents64` and correctly
  classifies file-vs-directory entries; a real two-child `wait_any`
  fires `pidfd_open` for each pid and picks the first to finish.

This closes platform-linux's raw-syscall coverage — no remaining gaps
between what `track-p` claims to cover and what it actually routes
through rusty_libc.

## Phase 5 — Net surface (D16)

**Lands here** — the biggest surface by consumer count: shh, rusty_tail,
rusty_rdp, and rusty_llama's optional server all want it, and none of
them need TLS in the trait (all four bring their own wire crypto or
inject TLS separately). Shape: TCP connect/listen + `set_nodelay`, UDP
datagram, Unix sockets (incl. mode + stale-cleanup bind).

Cheapest real convergence to prove it: **rusty_rdp**, whose `net.rs` is
already generic over `Read + Write` — supplying the PAL's stream type
as `S` is close to a no-op. Do that convergence PR immediately after
the surface lands, before shh/rusty_tail (which also need Phase 6/7/8
pieces and would otherwise block the "does this trait work" signal).

**Landed (TCP slice) 2026-07-19.** Scoped to TCP only this slice — UDP
datagram and Unix sockets (mode + stale-cleanup bind) deferred, the same
phased-slicing judgment call the Fs surface made for symlinks/`access`.
Added:

- `platform::net::{Net, TcpStream, TcpListener}` — a stateless
  capability-factory trait, the same shape as `Spawner`/`Dir`.
  `TcpStream`/`TcpListener` are `Send` (unlike `Dir`/`Child`): the
  accept-then-hand-off-to-a-worker-thread pattern is this surface's
  entire reason for existing, caught as a real compile error by a live
  scratch test before the bound was added, not a hypothetical.
- Linux: raw `libc` socket calls (`socket`/`bind`/`listen`/`accept4`/
  `connect`/`getsockname`/`getpeername`/`setsockopt`). Not track-p-gated
  at all — one implementation for both configurations, `fsync`'s
  precedent: sockets were never in rush's required surface per
  rusty_libc's own `DESIGN.md`, so there's nothing to route through it
  here. Strace-verified: a real loopback round trip fires
  `socket`→`setsockopt(SO_REUSEADDR)`→`bind`→`listen`→`getsockname` on
  the listen side and `socket`→`connect`→`setsockopt(TCP_NODELAY)` on
  the client side, with `accept4` on the accepting thread.
- Windows: raw Winsock2 (`WSAStartup`/`socket`/`bind`/`listen`/`accept`/
  `connect`/`getsockname`/`getpeername`/`setsockopt`/`recv`/`send`).
  `WSAStartup` is called lazily, exactly once, via `std::sync::Once`,
  deliberately with no matching `WSACleanup` — the OS tears down Winsock
  state at process exit regardless, and racing `WSACleanup` against
  in-flight sockets on other threads at shutdown is a real hazard
  "never clean up" avoids (mio/tokio/std's own Windows networking make
  the same call). Cross-compile-checked only — no windows-latest CI run
  has exercised this path yet, unlike every other backend piece landed
  so far.
- Mock: an in-memory duplex-channel implementation, the same "real
  behavior, no OS calls" contract `MockDir` has for the filesystem —
  a process-global registry of listening addresses plus a per-connection
  `mpsc` channel pair, with real connection-refused/addr-in-use/
  end-of-stream semantics.

**Landed (Unix sockets slice) 2026-07-20.** The deferred half of the TCP
slice — `platform::net::{UnixStream, UnixListener}` plus
`Net::unix_connect`/`unix_listen`, mirroring `TcpStream`/`TcpListener`'s
shape minus `set_nodelay` (no Nagle buffering on `AF_UNIX` to toggle) and
with `PathBuf` addresses standing in for `SocketAddr` — including a
third, TCP-never-has legal outcome an `AF_UNIX` peer can report:
`Ok(None)` from `peer_addr`/`local_addr` for an unnamed (anonymous)
endpoint. Added:

- Linux: `socket(AF_UNIX, SOCK_STREAM|SOCK_CLOEXEC)` +
  `bind`/`connect`/`accept`/`getsockname`/`getpeername` over a hand-packed
  `sockaddr_un` (the trailing `sun_path` array has no portable fixed
  offset the way `sockaddr_in`'s `sin_port` does, so the offset is
  measured once via `addr_of!`, the same technique `rename`'s
  `FILE_RENAME_INFORMATION` construction established). `unix_listen`
  narrows the freshly bound socket file to `0600` with a `chmod` right
  after `bind` — the mode-0600 half of D16's agreed shape (rusty_tail's
  LocalAPI, shh's agent socket), since a bare `bind` otherwise leaves the
  file at whatever the process umask allows.
- Windows: the same Winsock plumbing the TCP slice already pays for
  (`WSAStartup` once, `SOCKADDR_UN`/`afunix.h`'s 108-byte layout) —
  `socket(AF_UNIX, SOCK_STREAM)` + `bind`/`connect`/`accept`. No
  mode-narrowing step: Winsock's `AF_UNIX` bind has no POSIX-chmod
  equivalent, so the bound file is left at the filesystem's own ACL
  defaults instead of forcing an owner-only mode nothing in Windows'
  model enforces the same way.
- Both backends: real stale-cleanup bind, the other half of D16's agreed
  shape. A `bind` onto a path a live listener already holds and a path
  left behind by a listener that died without unlinking it hit the
  identical `AddrInUse` wall — the kernel/Winsock can't tell the two
  cases apart at bind time — so `unix_listen` resolves the ambiguity
  itself with a throwaway probe connect: `ECONNREFUSED`/`WSAECONNREFUSED`
  means stale (unlink and retry the bind exactly once), a successful
  probe means live (left untouched, `AddrInUse` surfaces normally). No
  caller-side unlinking needed. (An earlier pass of this slice shipped
  without this — caller-must-unlink-first — and got corrected before
  merge once review caught that it silently dropped the exact behavior
  D16 named this slice for.)
- Mock: `MockUnixListener`/`MockUnixStream` extend the registry-plus-
  channel-pair pattern `MockNet`'s TCP side already established, with the
  same real connection-refused/addr-in-use semantics; a listener's `Drop`
  frees its path in the registry so a later `unix_listen` on the same
  path succeeds — mirroring "dropping the first listener frees the
  address for reuse" from the TCP side.

**Landed (Unix-socket parity suite) 2026-07-20.** The one follow-up
this slice's own note above flagged as open: `net_parity.rs`
(kept textually identical across both crates) gained
`assert_unix_behavior` — connect/accept, the unnamed-peer case a plain
`unix_connect` client always hits, refusal once the listener drops,
and stale-cleanup bind reclaiming the leftover path afterward,
strace-verified live on Linux end to end (the real `bind` →
`EADDRINUSE` → probe `connect` → `ECONNREFUSED` → `unlinkat` → `bind`
sequence, not just the unit-level mock/real-backend tests each already
had). `docs/behavior/net.md`'s spec itself already covered Unix
sockets in full before this — only the shared cross-backend assertion
was the gap.

**Landed (UDP datagram slice) 2026-07-20.** The third and final D16
slice — `platform::net::UdpSocket` plus `Net::udp_bind`, named for
rusty_tail's magicsock transport. No listener/stream split unlike
TCP/Unix: one connectionless socket both sends and receives, addressed
per call via `send_to`/`recv_from` rather than a fixed peer from
`connect`/`accept` — and no `set_nodelay` (Nagle's algorithm has
nothing to do with a connectionless protocol). Designed and
implemented directly rather than through another workflow fan-out,
given what the Unix sockets slice's trait-design delegation cost last
time. Added:

- Linux: `socket(AF_INET/AF_INET6, SOCK_DGRAM|SOCK_CLOEXEC)` +
  `bind`/`sendto`/`recvfrom`, reusing the TCP slice's `to_sockaddr`/
  `from_sockaddr`/`local_addr` helpers as-is (`getsockname` and
  sockaddr packing don't care about socket type).
- Windows: the same Winsock plumbing (`WSAStartup` once) — `socket(...,
  SOCK_DGRAM, 0)` + `bind`/`sendto`/`recvfrom`, same helper reuse.
- Mock: a third process-global registry (independent from the TCP and
  Unix ones — real OSes keep `SOCK_STREAM`/`SOCK_DGRAM`/`AF_UNIX` bind
  spaces separate too) keyed by bound address, holding each socket's
  `Sender` half so other sockets' `send_to` can reach it; `recv_from`
  reads from the receiver half. `send_to` to an unbound address is a
  genuine no-op — dropped, not an error — the in-memory analog of a
  real fire-and-forget `sendto`.
- **The one genuinely new behavior across the whole Net surface**:
  `send_to` never fails because nothing is bound at the destination —
  there is no handshake to fail the way `tcp_connect`/`unix_connect`
  have. Strace-verified on Linux: a real `sendto` to a since-closed
  socket's old port returns success (the full byte count), not an
  error.
- `docs/behavior/net.md` and both `net_parity.rs` files (kept textually
  identical) get UDP's own assertion function, separate from TCP's —
  the same judgment call `assert_fs_behavior` made once for
  symlinks/access, made here because UDP's behavior barely overlaps
  with TCP's at all.

This closes out D16's original four-consumer survey — TCP, Unix
sockets, and UDP datagram all landed, and with the Unix-socket parity
suite landed too (see the note above), Net's own parity coverage is
complete across all three slices.

**Landed (raw-fd + non-blocking escape hatch) 2026-07-21** — filed as
rustils#41 by rusty_tail's own maintainer, mid-design on `rusty_tokio`
(a hand-rolled async runtime): wants to sit on `platform-linux`'s
sockaddr-packing/error-mapping/stale-socket-cleanup logic rather than
reimplement it, but the net surface was blocking-only and fully
encapsulated — no fd accessor, no way to toggle `O_NONBLOCK`, nothing
reachable to register with a reactor. Added to the five concrete Linux
socket types (`LinuxTcpStream`/`LinuxTcpListener`/`LinuxUnixStream`/
`LinuxUnixListener`/`LinuxUdpSocket`): `AsFd`/`AsRawFd`, `From<OwnedFd>`,
`set_nonblocking` (`fcntl(F_GETFL)`/`fcntl(F_SETFL)`), and concrete
`connect`/`bind`/`accept` constructors that call the exact same
`sysnet::` functions `Net`'s trait impl now just wraps — without the
constructors, the accessors would have been unreachable dead code,
since `Net::tcp_connect` and friends only ever hand out `Box<dyn
TcpStream>`, which erases the concrete type with no safe way back
(object-safe, not `Any`). Deliberately inherent-impl-only, not a change
to the object-safe `platform::net` traits themselves — mirrors
`LinuxFile`/`LinuxDir`'s existing std-interop precedent (`fs.rs`)
rather than inventing a new one. Live-verified (`net_nonblocking.rs`):
`O_NONBLOCK` actually flips in the kernel (checked via a raw `fcntl`
call bypassing this crate's own code), a non-blocking `accept`/
`recv_from` with nothing pending returns `WouldBlock` immediately
rather than hanging, and `From<OwnedFd>` adopts a socket built entirely
through `std`, not this crate's own connect/bind path. `x86_64`/Linux
only for now, matching the issue's own scope — `platform-windows`
untouched.

**Landed (Windows raw-socket + non-blocking escape hatch) 2026-07-21**
— rustils#59, the `platform-windows` half of the gap #41 left: the
same `rusty_tokio` consumer scoping a Windows/IOCP reactor backend
(`rusty_tokio#6`) hit the identical wall — no socket-handle accessor,
no way to toggle non-blocking mode. Added to the five concrete Windows
socket types (`WindowsTcpStream`/`WindowsTcpListener`/
`WindowsUnixStream`/`WindowsUnixListener`/`WindowsUdpSocket`):
`AsRawSocket` (raw-handle exposure only — no `AsSocket`/ownership-
transfer interop, since `sysnet::OwnedSocket` is this crate's own
newtype rather than std's `std::os::windows::io::OwnedSocket`, and
nothing has asked for adopting an externally-created socket on Windows
the way `From<OwnedFd>` does for Unix), `set_nonblocking`
(`ioctlsocket(FIONBIO, ...)`, Winsock's equivalent of
`fcntl(F_SETFL, O_NONBLOCK)`), and the same concrete
`connect`/`bind`/`accept` constructors #41 added on Linux, for the
identical reason (otherwise `AsRawSocket`/`set_nonblocking` would be
unreachable behind `Net`'s boxed-trait-only methods). Cross-compile-
checked only (`net_nonblocking.rs`, mirroring the Linux suite's own
test names) — Winsock's `ioctlsocket(FIONBIO, ...)` is set-only with
no matching query call, so verification there is behavioral (a
would-otherwise-block `accept`/`recv_from` returns `WouldBlock`
immediately) rather than a flag read-back, and real execution needs
the `windows-latest` CI leg, same caveat every `platform-windows`
addition carries until it runs there.

**Landed (`TcpStream::set_read_timeout`) 2026-07-20.** The one thing
that reopened this "done" domain: starting the rusty_rdp convergence
this phase's own note above flags as cheapest (its `net.rs` driver is
already generic over `Read + Write`) surfaced a real gap in
`platform` itself before any code changed in rusty_rdp's own repo.
rusty_rdp's own
`examples/connect.rs` idles a read loop out via
`std::net::TcpStream::set_read_timeout`, a capability
`platform::net::TcpStream` had no equivalent for. Added
`set_read_timeout(&self, timeout: Option<Duration>) -> Result<()>` —
an idle timeout (each `read` gets its own fresh clock, not a per-call
deadline), `None` blocking indefinitely (the prior, only, behavior).
Linux: `setsockopt(SOL_SOCKET, SO_RCVTIMEO, &timeval, ...)`. Windows:
the same option name but a plain millisecond `DWORD` instead of a
`timeval` — a wire-representation difference, not a behavior one, so
not a registered divergence. Mock: a per-instance `Cell<Option
<Duration>>` plus `mpsc::Receiver::recv_timeout`, a real timeout, not a
no-op. A timeout expiring is deliberately **not** pinned to one
`ErrorKind` (`WouldBlock` or `TimedOut`, backend-chosen) — the same
ambiguity `std::net::TcpStream::set_read_timeout` itself documents
(Linux's `SO_RCVTIMEO` expiring is indistinguishable from `EAGAIN` at
the errno level), so every real caller already has to check both, and
this trait doesn't pretend to resolve what the OS itself can't.
Deliberately scoped to `TcpStream` only — `UnixStream`/`UdpSocket`
have no forcing consumer for it yet (RFC v2 §3). Strace-verified on
Linux: a real 100ms timeout fires the `setsockopt` and the subsequent
`read` genuinely returns after ~103ms with `WouldBlock`. Caught a
real, unrelated pre-existing bug in the process: the Unix-socket
parity suite's `assert_unix_behavior` used a path keyed only on pid,
so `mock_unix_conforms` and `linux_unix_conforms`/`windows_unix_conforms`
running concurrently in the same test binary shared the identical
literal path — mock's own harmless-looking cleanup unlink could
intermittently delete the real backend's *live* socket file mid-test.
Fixed by keying the path on a per-backend label too.

**Landed (`platform-macos` backend, net-only) 2026-07-21** — rustils#48.
A third `Net` implementor forced the same way this whole phase was:
`rusty_tail`'s `rusty_tokio` async runtime needed a kqueue reactor
backend for macOS/BSD and had no `platform-macos` to sit its socket
lifecycle on, so it hand-rolled `MacosTcpStream`/`MacosTcpListener`/
`MacosUdpSocket` a second time against raw `libc`
(`src/io/socket/macos.rs`) — the exact duplication this phase's Linux
slice already solved once. Filed as rustils#48 asking for scope/priority
input rather than assuming a full backend was wanted; scoped down to
net-only (mirrors just the pieces `rusty_tokio` actually uses) with
cross-compile-check-only validation (no macOS runner in this workspace's
CI), both decided explicitly before implementation rather than guessed.
`crates/platform-macos` mirrors `platform-linux`'s `ffi`→`sys`→trait-impl
layering exactly, including the rustils#41 `AsFd`/`AsRawFd`/
`From<OwnedFd>`/`set_nonblocking`/concrete-constructor surface from day
one (the issue's own request, not deferred as a follow-up this time).
Three real BSD-vs-Linux syscall differences, all mechanism-only —
zero behavioral divergence at the `platform::net` boundary, so
`docs/divergences.md` gains no new entry for this slice:

- No `SOCK_CLOEXEC`/`SOCK_NONBLOCK` socket-type flags on Darwin —
  `fcntl(F_SETFD, FD_CLOEXEC)` right after `socket`/`accept` stands in
  for the former (a fork+exec race the atomic Linux flag avoids and
  this crate's own doc comment names rather than hides); the rustils#41
  `set_nonblocking` escape hatch already covers the latter via
  `fcntl(F_SETFL, O_NONBLOCK)`, unchanged in shape from the Linux slice.
- No `accept4(2)` — plain `accept(2)`, then the same post-creation
  `fcntl(F_SETFD, FD_CLOEXEC)`.
- `sockaddr_in`/`sockaddr_in6`/`sockaddr_un` all carry a leading
  `sin_len`/`sin6_len`/`sun_len` byte Linux's variants don't. Built via
  `zeroed()` + explicit field assignment rather than a full struct
  literal specifically so this extra field is set without ever needing
  to be named at a call site — the shape rustils#48's own issue body
  suggested and this slice adopted as-is.

Cross-compile-checked only (`cargo check`/`clippy --target
x86_64-apple-darwin`, plus the existing Linux-native and
`x86_64-pc-windows-gnu` legs unaffected — `platform-macos` compiles to
an empty crate on both, the same `#![cfg(target_os = "…")]` discipline
`platform-linux` already established for non-Linux hosts): no macOS
runner exists in this workspace's CI yet, so real hardware verification
is future work, not claimed here. `docs/behavior/net.md`'s parity-suite
list and `docs/extraction-map.md`'s D16 entry are both updated for this
slice; `fs`/`process`/`security`/`term`/`signals` stay out of scope
until a consumer forces them (RFC v2 §3), same as every other surface.

**Widened (`platform-macos` → `platform-bsd`, generic BSD) 2026-08-02**
— rustils#86. The same forcing function, one OS over: `rusty_tokio`#116
wanted the `kevent` reactor on FreeBSD/OpenBSD, and the reactor itself
was reusable (all the BSDs implement `kqueue`) but had no `platform`
socket layer to sit on for anything but Darwin — so it was about to
hand-roll a *third* socket lifecycle against raw `libc`, the exact
duplication #48 was filed to stop. Gate widened from
`target_os = "macos"` to
`any(macos, freebsd, openbsd, netbsd, dragonfly)`, crate and every
public type renamed (`MacosNet` → `BsdNet`, etc.), PAL group bumped
0.20.0 → 0.21.0 per `docs/versioning.md` §2.

The interesting finding is that **the implementation needed no
change at all**, and why. Of #48's three documented BSD-vs-Linux
differences, only the third — the `sin_len`/`sin6_len`/`sun_len`
leading byte — is genuinely universal (it's the 4.4BSD sockaddr layout
every BSD inherited). The first two are *Darwin's alone*: FreeBSD,
OpenBSD, NetBSD and DragonFly all have `SOCK_CLOEXEC`/`SOCK_NONBLOCK`
and `accept4`. So the Darwin-shaped code #48 wrote is the
*intersection* of the five targets rather than a Darwin special case —
correct everywhere, optimal only on macOS. `ffi::libc_surface` stays
that intersection deliberately: admitting `SOCK_CLOEXEC`/`accept4`
would compile on four targets and break the fifth. The price is the
fork+exec race between `socket`/`accept` and the follow-up
`fcntl(F_SETFD, FD_CLOEXEC)` — a race four of the five could have
avoided atomically, accepted to keep one code path instead of two, and
documented at `sys::net::set_cloexec` along with exactly what closing
it would take.

Verified, not assumed — which was the issue's own condition, on the
strength of #48's own history (its `macos` job caught the `AF_UNIX`
`len` bug that cross-compiling could not). Real FreeBSD and OpenBSD VM
jobs (`vmactions/*-vm`) now run the full suite alongside the
`macos-latest` job, and `cross-compile-check` gained clippy legs for
`x86_64-apple-darwin`/`-freebsd`/`-netbsd`. **Coverage is honestly
partial and labeled as such**: three of the five gated targets execute
(macOS, FreeBSD, OpenBSD), NetBSD compiles but never runs, DragonFly
does neither — tier 3, no prebuilt `std`, no runner. Those last two sit
in the gate by inheritance from FreeBSD's socket surface, an inference
rather than a measurement, and `platform-bsd`'s own `lib.rs` says so.

## Phase 6 — Security surface (D15)

**Lands here**, staged narrow-to-wide:

1. `fill_random` / CSPRNG — trivial, self-contained. Retires
   rusty_rdp's five hand-rolled `/dev/urandom` reads. Do this first;
   it's a same-day PR with an immediate convergence.

   **Landed 2026-07-20** — `platform::security::Csprng::fill_random`.
   Linux draws from the raw `getrandom(2)` syscall (strace-verified: a
   300-byte request returns the full buffer in one call, flags `0`,
   distinct from glibc's own internal stack-canary `getrandom` call at
   process start) rather than opening `/dev/urandom` as a file, so a
   future Landlock/seccomp policy (item 3 below) has no `fd` to have
   denied. Windows uses `BCryptGenRandom` with
   `BCRYPT_USE_SYSTEM_PREFERRED_RNG` (a null algorithm handle), the
   modern replacement for the deprecated `CryptGenRandom`, added a new
   `Win32_Security_Cryptography` `windows-sys` feature. `platform-mock`'s
   `MockCsprng` is a small seeded xorshift64* generator — deterministic
   for reproducible tests, but still varying, so a caller that never
   actually reads `buf` doesn't pass silently. See
   `docs/behavior/security.md` for the full contract. The rusty_rdp
   consumer wiring itself (retiring the five `/dev/urandom` reads in
   `src/krb5/kdc.rs`) is a follow-up in that repo, not this PR — the
   same two-step shape the Net surface's TCP slice and
   `platform_net.rs` adapter followed.
2. `CredentialStore` (get/set/available, disabled-mode escape hatch) —
   modeled on nexus's `keyring`-backed vault.

   **Checked, held 2026-07-20** — nexus's `CredentialVault`
   (`nexus-security/src/credential.rs`) is a complete, working,
   tested wrapper over the third-party `keyring-rs` crate. No gap, no
   TODO referencing rustils, no expressed desire to migrate — donor
   material only, not a live consumer. Building this now would be
   exactly the speculative build RFC v2 §3's consumer gate exists to
   prevent. Revisit if/when nexus (or another repo) actually needs it.

   **Landed anyway, 2026-07-23** — the owner's explicit call, same
   posture as item 3's `Sandbox` (built without a confirmed consumer,
   deliberately). Split across three PRs given its size: rustils#76
   (`platform::security::CredentialStore` trait, real Windows
   Credential Manager backend, faithful mock, and the
   `NullCredentialStore` disabled-mode escape hatch the roadmap item
   names — **landed**), rustils#77 (a hand-rolled D-Bus client
   transport for Linux — no existing D-Bus dependency, matching this
   repo's raw-bindings philosophy over pulling in `keyring-rs` the
   way the donor does — **landed**: `platform_linux::sys::dbus`,
   little-endian message marshaling/unmarshaling for the type-system
   subset Secret Service needs, `AF_UNIX` connect [both real-path and
   Linux abstract-namespace addressing], the SASL `EXTERNAL`
   handshake, and the mandatory post-auth `Hello` registration call a
   first implementation attempt initially missed — live-verified
   against a real `dbus-daemon --session` spawned as a CI test
   fixture, not just round-trip unit tests), rustils#78 (the Secret
   Service protocol on top of #77, wired into the real Linux
   implementation — **landed**: `platform_linux::sys::secret_service`,
   `LinuxCredentialStore` now delegates to it in place of the #76 stub.
   Live-verified the same way #77 was, against a real `dbus-daemon
   --session` + `gnome-keyring-daemon --unlock --components=secrets`
   pair spawned as a CI test fixture — round-trip, per-account
   isolation, replace-on-set, and binary payloads, not just the
   unreachable-bus degrade path). `get`/`set`/`available` only,
   exactly the roadmap's documented shape — no `delete`, not
   freelanced beyond what was asked. See
   `docs/behavior/security.md` for the full contract.

3. Sandbox policy (Landlock + seccomp on Linux, `Unsupported` stubs
   elsewhere initially) — modeled on nexus's `os_sandbox.rs` and shh's
   `privsep.rs` (fork-before-runtime, `NO_NEW_PRIVS`, rlimit,
   credential-drop-with-regain-check). The largest, most
   design-sensitive piece in this phase; do it last and expect an
   RFC-level discussion of the per-OS confinement story (macOS
   seatbelt and Windows restricted tokens have no donor yet).

   **RFC-level discussion held 2026-07-20** — see
   `docs/design-discussion-sandbox.md`, written after verifying both
   donors' actual source (not their own docs' framing). Key finding:
   nexus's and shh's "sandbox" material are two different problems —
   nexus needs process *confinement* (narrow what a process can
   touch), shh needs privilege-separation *isolation* (keep a secret in
   one process while another does risky work) — that don't share a
   trait shape. shh's pattern also doesn't fit `platform::process`'s
   current shape at all (no raw `fork`/`setuid`/`prctl`/`socketpair`
   exposed anywhere in this crate) and stayed out of scope.

   **Landed (confinement half only) 2026-07-20** —
   `platform::security::Sandbox::confine_filesystem`/`block_inet_sockets`,
   built without a confirmed live consumer as an explicit owner call
   (accepting the same speculative-build risk `CredentialStore` above
   was held for, deliberately, since nexus's implementation is the
   closest thing this repo has to a validated design to mirror).
   Linux: raw Landlock syscalls (ABI v1, no libc wrapper exists) for
   filesystem confinement — live-verified via strace that the exact
   correct access-flag set and struct size reach the kernel
   byte-for-byte, though full enforcement couldn't be exercised in
   this session's own sandboxed environment (`landlock_create_ruleset`
   returns `ENOSYS` there — confirmed via a raw C probe bypassing this
   crate's own code entirely, so an environment limitation, not an
   implementation bug); a hand-written seccomp-BPF filter for
   `block_inet_sockets` — fully live-verified working (`TcpListener`/
   `UdpSocket::bind` fail with `EPERM` after enforcement,
   `UnixListener::bind` unaffected), `x86_64`-only for now (the
   filter's mandatory architecture check is `AUDIT_ARCH_X86_64`-
   specific). Both report a three-way `SandboxStatus`
   (`Enforced`/`NotEnforced`/`Unsupported`) rather than silently
   degrading — `NotEnforced` exists specifically because nexus's own
   code showed nothing stops a caller from ignoring it. Windows and
   `platform-mock` report `Unsupported` unconditionally — no donor
   for restricted tokens/AppContainer, and no honest way to fake
   kernel-level confinement in memory. See `docs/behavior/security.md`
   for the full contract.

   **Re-verification attempt, 2026-07-21** — a fresh session tried again
   in a different sandboxed environment (a Firecracker microVM, not the
   prior session's container) and hit the same `ENOSYS`, but ran the
   root cause down further this time: `Seccomp: 0` on the process (no
   seccomp filter in play, ruling out a container policy denying the
   syscall) and `/proc/config.gz` shows `CONFIG_SECURITY_LANDLOCK is not
   set` — Landlock is compiled out of this kernel entirely, on a 6.18.5
   kernel that otherwise fully supports it (added in 5.13). Confirmed
   across two independent sandboxed sessions, of two different
   isolation kinds (container-based and Firecracker-microVM-based),
   that Landlock enforcement cannot be live-verified in this execution
   environment family. `security_sandbox.rs`'s
   `confine_filesystem_enforces_read_write_boundaries` test still passes
   by exercising only its `NotEnforced` degrade-path branch (early
   return before any of the actual enforcement assertions) — the
   `Enforced` branch remains unexercised in any session to date. Closing
   this out needs a bare-metal host or VM with `CONFIG_SECURITY_LANDLOCK=y`,
   not just another sandboxed session — treat repeating this same probe
   in another container/microVM as a known dead end rather than
   something worth re-trying.

## Phase 7 — PTY surface (D13)

**Status: landed, both halves, 2026-07-23** — see the two "Landed"
entries below for the full record. The paragraph immediately below
predates that and is kept as history, per this document's own "mark
items done in place, don't delete the history" rule — do not read it
as current status.

**Blocked on a named consumer** (historical, superseded below) — this
was the one item on this roadmap that wasn't ready to schedule. shh and
rusty_term both have working
donor code (openpty+TIOCSCTTY vs ConPTY), but neither is *itself* the
forcing consumer in the §3 sense: shh doesn't need to give up its own
pty.rs to unblock anything, and rusty_term's PTY hosting is scoped
deliberately out of its converged Terminal backend (see D9's oracle
note). Real candidates: a job-control-capable rush-interactive
(Phase 5 rush gate), or rusty_naner hosting rusty_term through a PAL
PTY instead of a raw subprocess launch. **Action: raise with the owner
before starting** — don't build speculatively.

**Raised with the owner, 2026-07-23** — built anyway, the owner's
explicit call, same posture as `CredentialStore`/`Sandbox`'s
confinement half: still no confirmed live consumer (both candidates
above stay hypothetical), accepted as the speculative-build risk those
two slices were already held under. Design pass held first per this
entry's own "expect an RFC-level discussion" instruction — see
`docs/design-discussion-pty.md` for the full donor-shape reconciliation
(one atomic `Pty::spawn` rather than separable open/attach steps, since
Windows's `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE` structurally can't
attach after the fact; `Ok(0)`-at-EOF unifying Unix's `EIO` and
Windows's broken-pipe signal). Split across two issues given its size,
the same reasoning `CredentialStore` was split across three: rustils#82
(trait shape + Linux backend + mock), rustils#83 (Windows ConPTY
backend, depends on #82's shape landing first).

**Landed (part 1/2: trait + Linux backend + mock), 2026-07-23** —
`platform::pty::{Pty, PtyMaster}`, `platform_linux::{LinuxPty,
LinuxPtyMaster}`, `platform_mock::{MockPty, MockPtyMaster}` (rustils#82).
Notably: shh's own `fork`+`TIOCSCTTY` mechanism was **not** ported as-is
— raw `fork` stays parked behind its own separate roadmap decision
above, so the Linux backend reaches the identical outcome (child ends up
session leader with the pty slave as its controlling terminal) through
`posix_spawn`'s own `POSIX_SPAWN_SETSID` flag plus a file action that
opens the slave by pathname, rather than reopening the async-signal-
safety hazard `sys::spawn` exists to close. Live-verified against
`/proc/<pid>/stat` kernel ground truth (`sid == pid`, `tty_nr != 0`),
not just a successful `posix_spawn` return — see
`docs/behavior/pty.md` for the full contract. Windows (issue #83) not
yet landed.

**Landed (part 2/2: Windows ConPTY backend), 2026-07-23** —
`platform_windows::{WindowsPty, WindowsPtyMaster}` (rustils#83).
`CreatePseudoConsole` wired to the child at `CreateProcessW` time via
`STARTUPINFOEXW`/`PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE` — the only way to
attach one at all, unlike Unix's separable-in-theory (though not taken)
open/attach steps. Not grouped — no Job Object, so `kill_tree` on a
pty-hosted `Child` is `Unsupported` on Windows, a deliberate scope
reduction. Deliberately **not** the suspended → assign → resume sequence
`Spawner::spawn`'s `NewGroup` path uses, matching Microsoft's own sample
(which doesn't suspend). `read`/`write` are ordinary blocking
`ReadFile`/`WriteFile` on the two pipe handles ConPTY's master genuinely
is. Five real bugs surfaced only through live CI runs, not local
cross-compile-checking: the `UpdateProcThreadAttribute` value/address
mixup, a `sys::pty::close` drain-loop deadline check that only ran in
one branch, and — the one that took seven CI rounds to isolate, since
`CREATE_SUSPENDED` removal, Job Object removal, test-concurrency
serialization, and a shell-vs-no-shell diagnostic all failed to change
the failure signature — every child's real console output was reaching
the *spawning* process's own ambient console instead of the pseudo
console's pipes, 100% reproducibly, and finally a documented-contract
gap only the first three bugs' fix exposed. Root cause of the third,
confirmed against `microsoft/terminal` discussion #15814: when the
spawning process's own stdio is itself redirected (exactly `cargo
test`'s situation under a CI runner), the kernel duplicates those
redirected handles into the child regardless of
`PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE`; setting `STARTF_USESTDHANDLES`
(with null std handles) in `STARTUPINFOEXW.dwFlags` suppresses that
duplication and is the actual fix. Once real output started reaching
the pipes, a fourth, distinct gap surfaced: ConPTY's output pipe does
not spontaneously EOF when the last attached process exits (unlike a
Unix pty slave, which the kernel closes automatically) — contradicting
`PtyMaster::read`'s own documented `Ok(0)`-on-child-exit contract.
`sys::pty::spawn_exit_watcher` is the fix: a background thread (the
one background thread this backend does need, despite the earlier
"no background thread for I/O" framing) that waits on a *duplicated*
process handle and calls bare `ClosePseudoConsole` once the child
exits, guarded by a shared `closed` compare-exchange against a
double-close race with `WindowsPtyMaster::drop` — whichever of "child
exits" or "caller drops the master" happens first wins, the other is a
no-op. A fifth bug, caught the same way as the rest — live CI, not
local checking — came from this fix's own first attempt: having the
watcher *drain* the output pipe before closing (matching `Drop`'s own
close path) raced a caller's concurrent `read()` for the same bytes on
the same handle, and wasn't a rare fluke — it consistently broke three
previously-passing tests down to only conhost's VT-negotiation bytes.
Calling bare `ClosePseudoConsole` instead (the watcher never touches
the output pipe at all) has no such race, at the cost of moving rather
than removing the original deadlock concern: `ClosePseudoConsole` can
itself block behind a full, undrained pipe, which can now stall the
watcher thread instead — accepted, since it's detached and never
joined — see the PR history for #83. New
divergence: `docs/divergences.md` #011 (single pollable fd on Linux vs
two non-pollable handles on Windows). CI-verified only (no Windows
execution available in the implementing session) —
`platform-windows/tests/pty.rs`, including a dedicated test that drops
an undrained master against a child producing far more output than a
pipe's default buffer holds, to actually exercise the teardown fix.
Both halves of the PTY surface (#82 Linux, #83 Windows) are now landed;
only macOS (no donor evidence) remains unaddressed within this phase's
scope. See `docs/behavior/pty.md` for the full contract.

## Phase 8 — Tun / virtual-link surface (D14)

**Lands here**, single named consumer (rusty_tail) — sufficient per the
same single-consumer precedent as rush itself. Linux `TUNSETIFF`
ioctls behind the `TunDevice` trait rusty_tail's own code already
anticipates (`ts-tun/src/lib.rs` comments defer this exact split);
Windows via wintun, `Unsupported` until a Windows consumer names itself.
Lower priority than Phases 1–6 only because it has one consumer instead
of several — not because the donor evidence is weak.

**Landed 2026-07-21** — `platform::tun::{Tun, TunDevice}`. Linux:
`/dev/net/tun` + `TUNSETIFF`, then `SIOCSIFADDR`/`SIOCSIFNETMASK`/
`SIOCSIFMTU`/bring-up via a throwaway `AF_INET`/`SOCK_DGRAM` socket —
the exact ioctl sequence `ts-tun/src/sys.rs` already hand-rolls.
Live-verified in this sandboxed environment (`/dev/net/tun` and
`CAP_NET_ADMIN` both genuinely present, confirmed with a raw C probe
before committing to live tests over cross-compile-check-only): a real
created interface, a real installed connected route, a real
kernel-routed outbound packet read back off the device, and a
hand-crafted, independently-checksummed inbound IP/UDP packet actually
delivered to a bound `UdpSocket` via `write`. Ships the same raw-fd
escape hatch (`AsFd`/`AsRawFd`/`set_nonblocking` on the concrete
`LinuxTunDevice`, not the object-safe trait) rustils#41/#42 established
for `Net`, since `ts-tun` needs to register the fd with tokio's own
reactor exactly like `ts-magicsock` did. Windows's `WindowsTun` reports
`ErrorKind::Unsupported` explicitly — `ts-tun` (the only named
consumer) is Linux-only, so there is no donor evidence for a Windows
shape yet. `platform-mock`'s `MockTun`/`MockTunDevice` don't simulate
kernel routing (no "other side" to fake, unlike `MockUdpSocket`) —
scriptable instead: a test queues bytes for `read()` and asserts
against everything `write()` recorded. See `docs/behavior/tun.md` for
the full contract. rusty_tail's own `ts-tun` convergence onto this is a
follow-up in that repo, not this PR.

**CI finding, same day** — the dev sandbox this landed against runs as
root with `CAP_NET_ADMIN`, but CI's hosted `ubuntu-latest` runner
executes the `test` job as an unprivileged user, so `tun_parity.rs`'s
live tests failed there with `PermissionDenied` on `TUNSETIFF` —
caught by CI itself, not by re-checking assumptions ahead of time (the
same class of gap the Sandbox slice's Landlock `ENOSYS` finding was,
just discovered a step later in the workflow this time). Fixed by
having the tests skip gracefully rather than fail when lacking the
capability (mirroring `security_sandbox.rs`'s `NotEnforced` degrade
path), plus a dedicated `sudo`-elevated CI step so CI still gets
genuine live coverage instead of only ever exercising the skip branch.

**Second CI finding, same day** — with the privileged step actually
running, `linux_tun_outbound_packet_is_readable_from_the_device` still
failed intermittently: the very first packet read off a freshly-up'd
interface was not always the crafted UDP datagram the test just sent —
a classic TUN-device gotcha where the kernel emits its own spontaneous
traffic (IPv6 router solicitation/neighbor discovery, most commonly)
on a newly-up'd interface, arriving ahead of whatever a caller sent.
Fixed in the test only (not `sys::tun`'s `create`/`configure`, which
already behave correctly — a real device genuinely offers no ordering
guarantee here, so a production consumer needs to filter anyway):
`device.read()` now loops, skipping any packet that doesn't match the
expected IPv4/UDP/destination-port shape, bounded at 16 attempts so a
genuine regression still fails fast rather than hanging.

## Phase 9 — Windowing + Registry/Config (nexus-only)

**Lands here** (trait) **+ nexus** (convergence), last and thinnest per
architecture.md's own assessment:

- Registry/Config first — nexus's `persistence.rs` (file-backed JSON +
  `dirs` paths) is a simple personality to formalize.
- Windowing last — nexus's window management is Tauri-mediated, not
  raw OS calls, so the PAL trait would be a thin pass-through with low
  near-term value. rusty_rdp's "display" is wire-encoding, not OS
  windowing, and does not force this surface (architecture.md
  correction, D-survey).

**Design pass held 2026-08-06** — `docs/design-discussion-windowing-registry.md`,
this phase's first real design pass (both bullets above were
provisional, never checked against nexus's actual source). Reading
`shell/src-tauri/src/{persistence,windows}.rs` directly found a
stronger verdict than either bullet: Windowing has *zero* OS-syscall
surface at all (no `windows-sys`/`x11`/`wayland`/`cocoa` anywhere in
nexus's Tauri crate — not "thin pass-through," no pass-through),
and Registry/Config's one real gap (known-directory resolution,
`app_config_dir`-equivalent) is answered by Tauri itself for this
consumer, not hand-rolled the way "`dirs` paths" implied — the same
"complete, working, no gap, no expressed desire to migrate" verdict
`CredentialVault` already got. Still not decided whether to close this
phase outright (matching `rusty_lsp`'s own "converges by doing
essentially nothing" precedent) or build the one narrow, generalizable
primitive found (known-directory resolution) speculatively anyway —
open questions in the linked document.

---

## Decided: fork/execve vs posix_spawn

Independent of every phase above — a design decision, not a sequencing
question. `docs/architecture.md` used to show raw `fork`/`execve` on
Layer 1 Linux as the target picture, disagreeing with current code's
`posix_spawn` (which outsources async-signal-safety to glibc);
recorded there as an open item and left for an owner call.

**Dug into 2026-08-06** — see `docs/decision-request-fork-execve.md`:
checked against rusty_libc's real current source rather than the
original framing at face value, and found "adopting rusty_libc's
fork+execve" describes donor material that doesn't exist yet (no
`fork`/`execve`/`clone` anywhere in that crate) — a two-repo project,
not a rustils-side flag flip. Option 3 (`memfd_create` alone, D11's
own prerequisite for a raw fork) landed the same day (Phase 3's own
entry, above), independent of the larger question.

**Decided the same day: option 1 — stay on `posix_spawn`
indefinitely.** `docs/architecture.md`'s target picture updated to
match (its own "Decided" section has the full reasoning): `posix_spawn`
already removes the async-signal-safe critical region by construction,
PTY hosting (D13) already independently chose the identical answer
under real pressure, and going raw would mean commissioning a new
hazard class in rusty_libc with no named consumer forcing it. This
item is now closed, not parked — revisit only if a real consumer or a
recovered rationale beyond Track P's own general "no glibc" aspiration
ever surfaces.

---

## How to use this document

Pick a phase (or an item inside one), get the go-ahead, do the work
through the standing PR workflow, then mark it done here with a
landed-note (mirror `extraction-map.md`'s style: date, PR/commit,
one-line summary) rather than deleting the entry. Convergence items
that land in other repos should still get a landed-note here, so this
stays the one place that shows the whole ecosystem's status at a
glance.
