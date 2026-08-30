# Design discussion — a shared pgid/session table (msys-parity subsystem 1, deeper scope)

Not a decision record. `docs/design-discussion-msys-parity.md` named
subsystem 1 as "a shared cross-process pgid/session table" in one
sentence and moved on — both subsystems scoped deeper since
(`docs/design-discussion-msys-signals.md`,
`docs/design-discussion-msys-stop-cont.md`) explicitly found real ways
to *avoid* needing it. Subsystem 1 is what's actually left unaddressed:
the piece that would give Windows a numeric process-group identity a
spawn can join, which is what real `setpgid`/`getpgid`/`tcsetpgrp`-style
job control needs and neither of the other two subsystems attempted to
provide. This document re-reads Cygwin's real implementation (not the
parent document's one-line summary) to scope it properly, the same
discipline applied to subsystems 2 and 3.

## Where this plugs into what's already registered

`platform::process::GroupSpec::JoinGroup(u32)`
(`platform/src/process.rs:103`) already exists and is already
documented as Windows-`Unsupported`: *"Windows has no numeric
process-group id a spawn can join... `Spawner::spawn` fails
`Unsupported` (divergence 008)."* This is the mirror image of
divergence 010 (`Spawner::adopt` — Windows *can* adopt an
already-running pid by handle; Unix structurally cannot once it's
exec'd). Subsystem 1 is the piece that would let Windows answer
`JoinGroup` for real instead of refusing it — not a new capability
invented for this document, a named gap this crate already tracks.

## What Cygwin actually built — not one table, a table *per pid*

The one-sentence "shared table" summary undersells the real shape.
`pinfo::init` (`winsup/cygwin/pinfo.cc:395`) does not open one big
array; it maps **one small shared-memory segment per pid**:

```cc
procinfo = (_pinfo *) open_shared (L"cygpid", n, h0, sizeof (_pinfo),
                                   shloc, created, sec_attribs, access);
```

`open_shared` (`mm/shared.cc:126`) is `CreateFileMappingW`/
`OpenFileMappingW` under a name built by `shared_name()`
(`mm/shared.cc:100`: `"%s.%d"` → `"cygpid.<pid>"`), inside a
per-installation NT object directory
(`\BaseNamedObjects\<shared_id><build-date>-<installation-key>`,
`mm/shared.cc:47` — the installation key is a hash scoping multiple
independent Cygwin installs on one machine from colliding). **This is
architecturally the same shape subsystem 2 already landed** — a
per-pid, name-derived-from-pid OS object a sender/reader opens by
knowing the target's pid, no monolithic table, no separate discovery
service — just a shared-memory segment instead of a duplex pipe.
Restating this because it changes the estimate: subsystem 1 is not a
new kind of infrastructure this codebase would be building from
scratch; it's the same per-pid-named-OS-object pattern subsystems 2/3
already established, applied to `CreateFileMapping` instead of
`CreateNamedPipe`.

## A real, different security posture than subsystems 2/3 — and why that's correct

`pinfo::init` builds its security descriptor as:

```cc
sec_user_nih (sa_buf, cygheap->user.sid (), well_known_world_sid, FILE_MAP_READ)
```

Owner: full access. Everyone (`well_known_world_sid`): **read-only**.
This is deliberately *not* the SID-scoped, owner-only ACL subsystem 2's
signal pipe needs — and the difference is correct, not an
inconsistency to resolve: reading a pid's pgid/session (the
`getpgid(2)`/`ps`-shaped question) is legitimately world-visible on
real POSIX systems today; *writing* it (`setpgid`) or delivering a
signal into a process are the operations that need same-user
restriction. A rustils port should keep this same read/write split
rather than defaulting every shared object in this family to
subsystem 2's owner-only posture out of consistency for its own sake.

## Two liveness notions — only one of them is trustworthy alone

`_pinfo::exists()` (`pinfo.cc:597`) trusts the shared segment's own
cooperative `process_state` bits (`PID_EXITED`/`PID_REAPED`) — fast,
but only as good as the target's own cleanup, and wrong if that
process crashed hard. `_pinfo::alive()` (`pinfo.cc:603`) is the
independent check: `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION,
false, dwProcessId)` against the *real* Windows pid recorded in the
shared struct — succeeds only if the OS itself still has that process.
Cygwin keeps both because they answer different questions (has the
process told us it's gone vs. is it actually gone), and a rustils port
should keep the same split rather than collapsing to the cheaper,
spoofable one: a crashed or killed process leaves stale `process_state`
bits behind, and — the sharper hazard — Windows can and does **reuse
pids**, so a shared segment's `dwProcessId` field can start lying about
which real OS process it describes the moment the original exits and a
new, unrelated one gets the same pid. `Spawner::adopt` (rustils#47)
already established the pattern this needs: `OpenProcess` as the
ground-truth check, never trust in-band bookkeeping alone for "is this
still the process I think it is."

## Proposed rustils shape — deliberately narrower than Cygwin's own struct

`_pinfo` is Cygwin's general process-introspection backbone — it also
carries fds, cwd, environ, cmdline, signal masks, because it backs
`/proc` and `ps`, not just job control. **A rustils slice should not
copy that scope.** The narrow struct this actually needs:

```rust
#[repr(C)]
struct PgidEntry {
    pid: u32,
    pgid: u32,
    sid: u32,
    generation: u32, // see "reuse" below
    state: AtomicU32, // Running / Exited / Reaped — mirrors _pinfo's own bits, deliberately minimal
}
```

Four fixed-offset `u32`s plus one atomic state word — small enough
that plain atomic loads/stores on each field (the same "one atomic
store, safe-point consumption" discipline D6/`csignals.rs`/subsystem 2's
`PeerSignalSource` already established) cover every read/write this
slice needs, no separate mutex. Cygwin's own locking for pgid/sid
specifically wasn't found in the files this document actually read
(worth an explicit note: not verified, not assumed absent — a real
implementation would need to confirm this before trusting
lock-free atomics as sufficient).

**Naming**: Cygwin's elaborate per-installation NT-object-directory
scoping exists because multiple independent Cygwin *installations*
(different install roots, different versions) can coexist on one
Windows machine and must not collide. rustils is a linked library, not
an installed runtime with that multi-install-root shape — whether the
equivalent scoping question even applies, or a flat
`Local\rustils-pgid-<pid>` name is sufficient, is itself an open
question below, not assumed either way.

**No fixed-VA mapping**: Cygwin's `MapViewOfFileEx` into a reserved,
identical-across-processes address range (`mm/shared.cc:167`,
`SHARED_REGIONS_ADDRESS_LOW`/`HIGH`) is not obviously needed here — that
requirement usually comes from a shared structure holding raw pointers
meaningful across address spaces, or from decades of code assuming a
fixed layout; a from-scratch, POD-only Rust struct with no embedded
pointers has no such constraint. Worth confirming rather than assuming,
but the presumption in this document is that ordinary
`MapViewOfFile`-at-whatever-address-the-OS-picks is sufficient.

## What this would unlock, concretely — and what it wouldn't, yet

**Unlocks**: `GroupSpec::JoinGroup` becoming real on Windows —
spawn-time placement into an existing numeric pgid (write the child's
pid/pgid into a freshly created `PgidEntry` at spawn, the same
race-free "before the child's first instruction" shape `NewGroup`
already has, D1's own precedent — this is spawn-time placement, not
the harder post-hoc `setpgid(pid, pgid)` divergence 010 wrestled with
for Unix's `adopt` gap, so the exec-timing hazard that gap has doesn't
apply here).

**Does not unlock by itself**: real `tcsetpgrp`/foreground-group
routing. Cygwin's tty layer separately tracks "which pgid is the
foreground group" per tty (`shared_info::tty`, `local_includes/shared_info.h:47`)
and consults it when deciding which processes see a Ctrl-C-equivalent.
This table gives Windows processes a numeric pgid identity; wiring that
identity into `JobControlTerminal`-equivalent foreground-group
ownership on Windows (the trait this crate's own `docs/behavior/term.md`
documents as Unix-only, no Windows implementor at all) is a further,
still entirely unscoped step even once this subsystem lands. Stating
this plainly so this document isn't mistaken for "the last piece" —
it's a necessary piece, not a sufficient one.

**Also does not unlock**: `setpgid(pid, pgid)` as a standalone,
post-spawn call. This crate's `GroupSpec` is spawn-time-only on *both*
platforms today — there is no existing hook, Unix or Windows, for
"change an already-running child's group after the fact." A pgid table
makes that representable (write a new `pgid` into an existing
`PgidEntry`) but the portable API surface for it doesn't exist yet
either; out of this document's scope to invent it, flagged as a real
adjacent gap.

## Open questions for the owner, not decided here

1. **Still no named consumer** — same as every item in this family.
2. **Does rustils need Cygwin's multi-installation NT-object-directory
   scoping, or does a flat name suffice?** Depends on whether two
   independent copies of a `platform`-linked binary set (e.g. two
   different app versions) ever need to *not* see each other's pgid
   tables on the same machine — a real product question, not a
   technical one.
3. **Is per-field atomics sufficient, or does a real implementation
   need an explicit lock** for multi-field updates that must be seen
   atomically together (e.g. `pgid` and `generation` changing as one
   unit on reuse)? This document's "probably fine" is a guess, not a
   verified claim — the caveat above about Cygwin's own locking not
   being confirmed applies here too.
4. **Read-visibility scope**: Cygwin's world-readable ACL matches
   POSIX's own `getpgid` visibility. Does a rustils consumer actually
   want that (any local user can enumerate any `platform`-aware
   process's pgid/session), or is that more exposure than any named
   consumer has asked for — i.e. should this slice start owner-only
   like subsystem 2 and widen later only if something needs it?
5. **Sequencing relative to the still-unscoped `tcsetpgrp` wiring** —
   does this land as a self-contained `GroupSpec::JoinGroup` slice
   (useful today, e.g. for pipeline-stage grouping the way D1's Unix
   `JoinGroup` already serves), or does it wait until a foreground-group/
   `JobControlTerminal`-for-Windows design exists too, so the two land
   together rather than JoinGroup shipping years before anything
   consumes the table for job-control purposes?

## What this document does not decide

Whether to build this, whether a flat name or Cygwin's full
installation-scoped naming is warranted, or whether `JoinGroup` ships
before or after Windows-side `tcsetpgrp`/foreground-group wiring gets
its own design pass. Same posture as every document in this family:
input to an owner's call, not the call itself. With this document, all
four subsystems `docs/design-discussion-msys-parity.md` originally
named in one sentence each now have implementation-depth scoping —
none of them built, none of them consumer-gated yet.
