# Design discussion — MSYS2/Cygwin-level job control & signal parity on Windows

Not a decision record. This document scopes a question raised after the
console-acquisition slice landed (`docs/design-discussion-console.md`):
does rustils's Windows backend match what `msys-2.0.dll` (MSYS2, a
Cygwin fork) gives a POSIX-shaped program on Windows? **No** — and this
document is the reconciliation pass explaining exactly what the gap is,
grounded in MSYS2's real source, before anyone decides whether to close
it.

## A different kind of donor — read carefully before reusing anything below

Every donor in `docs/extraction-map.md` (rush, rusty_win32, rusty_term,
shh, rusty_lines, rusty_naner, …) is a `baileyrd/*` repo under a
permissive license, extracted under this project's normal rule: *port
semantics and tests, not code links* (extraction-map.md's own framing).

MSYS2 is not that. `winsup/cygwin` (the DLL implementing everything
described below) is **LGPLv3+** with a narrow linking exception that
covers programs *linking against* the Cygwin DLL — it grants no license
to copy or structurally derive its source into a separately-licensed
(here, MIT) project. This document was written after cloning
`msys2-runtime` (`github.com/msys2/msys2-runtime`, the Cygwin fork
MSYS2 ships) read-only to verify real mechanisms rather than work from
memory — the same "verified against source, not framing" discipline
`docs/design-discussion-sandbox.md` applied to nexus/shh. **If any of
this is ever implemented, it must be a clean-room re-derivation from
POSIX's own behavioral spec and public documentation** (what `setpgid`/
`kill`/`tcsetpgrp`/job control are contractually required to do), never
a port of Cygwin's actual code shape the way `sys::pty`'s ConPTY
sequence ports rusty_win32's. Citations below (real file/line numbers)
exist to make the *scope* concrete, not as a spec to translate line by
line.

## What MSYS2/Cygwin actually built

Four subsystems, all independent of anything Win32 or ConPTY natively
offers, all implemented entirely in the Cygwin DLL's own userspace code
that runs inside every Cygwin-aware process:

### 1. A cross-process, shared process/group/session table

`winsup/cygwin/local_includes/pinfo.h`'s `_pinfo` struct carries `pgid`/
`sid` fields (lines 89–90) alongside the Win32 pid — a POSIX process
tree (pid/pgid/sid, `has_pgid_children`, foreground/background
classification against a tty's pgid) modeled entirely in Cygwin's own
shared memory, independent of any Win32 job/process-tree concept.
`setpgid`/`getpgid`/`setsid`/`getsid` read and write this table
directly; Win32 has no numeric-pgid primitive to delegate to at all
(the same limitation `docs/divergences.md` #008 already registers for
`GroupSpec::JoinGroup`).

### 2. Inter-process signal delivery over a named pipe, not console events

`winsup/cygwin/sigproc.cc`: `sig_send` (line 591 onward) packages a
`sigpacket` and writes it to the *target* process's own signal pipe —
sized `PIPE_DEPTH = allocation_granularity / sizeof(sigpacket)` (line
32) — which a dedicated thread inside the target reads and dispatches
to that process's own signal-handling machinery (masks, `SA_RESTART`,
pending-signal queues). This is a full point-to-point, arbitrary-signal
delivery channel between any two Cygwin-aware processes — not the
three console-control-derived identities `WindowsSignalSource` maps
(`docs/divergences.md` #003) and not gated on sharing a console the way
`GenerateConsoleCtrlEvent` is.

`SuspendThread`/`ResumeThread` (referenced in `cygthread.cc`,
`exceptions.cc`, `thread.cc`) is how `SIGSTOP`/`SIGCONT` get emulated:
every thread in the target process is actually suspended/resumed, since
Windows has no process-wide stop primitive — the mechanism
`docs/divergences.md` #008 already flags as simply absent from this
crate's own `Signal` handling on Windows (`Signal::Kill`-only).

### 3. A fully userspace pty — not ConPTY, not console mode bits

`winsup/cygwin/fhandler/pty.cc` (4,623 lines) implements the pty master
and slave as a pair of named pipes it creates and names itself —
`\\.\pipe\msys-<installkey>-pty<N>-master-ctl`,
`...-to-master`/`...-from-master` (lines 906–2128, 3194, 3205) — with
zero dependency on `CreatePseudoConsole`/conhost (MSYS2/Cygwin predates
ConPTY by well over a decade; nothing in this file even guards on its
availability). The **line discipline itself** — echo, canonical-mode
editing, and which control characters generate which signals — is
Cygwin's own state machine (`line_edit`/`line_edit_maybe`, lines
2217–2825), not `ENABLE_VIRTUAL_TERMINAL_INPUT`'s VT-stream translation
or any Windows console-mode bit. `tcsetpgrp` (lines 1724, 2547) is
implemented at this same layer, directly against the shared pinfo
table's tty-pgid, not delegated to any OS primitive (there is none —
`docs/behavior/term.md`'s own `JobControlTerminal` doc already says so
for the Unix-only trait this crate has).

### 4. Session hangup wired through the same signal channel

`fhandler_pty_master`'s teardown path (`pty.cc:2172`) delivers `SIGHUP`
to the session leader via the same `kill`/`sig_send` machinery above
when the controlling pty goes away — an ordinary consequence of having
a real signal-delivery channel, not a separate mechanism.

## Why this isn't "extend `WindowsSignalSource`"

The gap isn't one missing method — it's an entire process-relationship
model this crate's Windows backend has deliberately not built, for
reasons already on record:

- **`GroupSpec::JoinGroup`** is `Unsupported` on Windows (divergence
  008) because Windows process groups are Job Object *handles*, not
  numeric pgids — MSYS2's answer is subsystem 1 above: stop relying on
  Win32 groups at all and maintain the numeric model itself, in shared
  memory, across every Cygwin-aware process on the machine.
- **`Signal` delivery** is `Kill`-only on Windows (divergence 008)
  because `TerminateProcess`/`TerminateJobObject`/
  `GenerateConsoleCtrlEvent` are the only OS-native asynchronous
  notifications that exist — MSYS2's answer is subsystem 2: build an
  arbitrary-signal channel that doesn't touch any of those OS
  primitives at all, plus `SuspendThread`/`ResumeThread` for the
  `Stop`/`Cont` pair no OS call can express.
- **`JobControlTerminal` has no Windows implementor** (this crate's own
  precedent, `docs/behavior/term.md`) because there is no
  `tcsetpgrp`-equivalent OS call — MSYS2's answer is subsystem 3: don't
  ask the OS, own the entire line discipline and answer `tcsetpgrp`
  from the same shared table subsystem 1 already maintains.
- **`ConsoleAcquisition`** (just landed) is a thin wrapper over real
  Win32 `AllocConsole`/`AttachConsole`/`FreeConsole` — MSYS2's ptys
  don't use the Win32 console subsystem for their own sessions at all
  (subsystem 3's named pipes replace it entirely); this crate's
  `ConsoleAcquisition` and MSYS2's pty model solve *different* problems
  that happen to share the word "console."

Put simply: every Windows divergence this crate has registered so far
(001, 003, 008) documents "the OS has no primitive for this." MSYS2's
answer to every one of them was the same move — **stop asking the OS,
build the POSIX model yourself, entirely in userspace, shared across
every cooperating process** — not a Windows API this crate has simply
failed to call yet.

## The participation boundary — MSYS2's own limitation, inherited by any port

None of subsystems 1–4 work with a process that isn't itself
Cygwin/MSYS2-aware: a plain native `.exe` spawned as a child gets none
of the pgid tracking, none of the signal channel, and is only reachable
via the same three console-control events this crate already has
(`pinfo.h`'s own `is_foreground_non_cygwin_process` check, line 142,
exists precisely to special-case that boundary). A rustils port would
inherit the identical boundary: **only processes spawned through this
same mechanism** — i.e. other `platform`-based processes opting in —
could participate in an emulated pgid/signal model. An arbitrary
already-running Windows process could never be `kill(2)`'d with a real
signal by this scheme any more than MSYS2 can `kill` a non-Cygwin
process today. Any future doc/implementation must state this boundary
explicitly rather than let a consumer assume POSIX `kill(pid, sig)`
semantics against *any* pid.

## Open questions for the owner, not decided here

1. **Is there a named consumer at all?** Every subsystem here is
   Track-P/D4-adjacent in ambition — this would be the single largest
   thing ever added to `platform-windows`, larger than PTY (D13) and
   Sandbox combined. `docs/design-discussion-pty.md` and
   `-sandbox.md` were both built on an explicit owner call to proceed
   *without* a confirmed consumer; this is a much bigger speculative
   bet to make the same call on. A concrete consumer (e.g. a
   `rush`-interactive port that needs real `fg`/`bg`/Ctrl-Z on Windows)
   would change this from "maybe someday" to "worth scoping a Phase."
2. **Does this become one `platform` capability or several independent
   ones?** The four subsystems are separable: the shared pgid/session
   table (1) is a prerequisite for both signal delivery (2) and
   `tcsetpgrp` (3), but a consumer could plausibly want *only* richer
   `Signal` delivery (2) without full job control (3) — e.g. sending
   `SIGTERM` to an arbitrary cooperating rustils child without ever
   needing `fg`/`bg`. Bundling all four into one landing (like Sandbox's
   confinement+privsep question 5) risks blocking whichever slice has a
   real consumer on whichever doesn't.
3. **Where does the shared table live, and what's its failure mode?**
   Cygwin's shared pinfo segment is scoped per-installation (keyed by
   an install-path hash baked into every pipe/mutex name — visible in
   the `%S` install-key format strings cited above). A rustils
   equivalent needs an answer to: what identifies "the same rustils
   installation" across independently-spawned processes (a fixed name?
   an env var inherited at spawn?), what happens to a stale entry after
   an ungraceful process death (Cygwin's answer involves reaping dead
   entries on next table touch — verified only by reading the source
   grounding this document, not something to copy structurally per the
   licensing note above), and whether a Windows service/detached
   process (no console at all) can participate.
4. **`SuspendThread`-based `SIGSTOP`/`SIGCONT` is a known-fragile
   primitive.** `SuspendThread` on a thread that holds a lock (loader
   lock, heap lock, CRT lock) can deadlock the *entire* stopped process
   the instant anything tries to touch that lock — a real, documented
   Windows hazard, not a hypothetical one. Cygwin accepts this risk;
   does rustils's own bar (`docs/design-discussion-pty.md`'s "needs
   live verification, not just inspection" standard, applied everywhere
   in this codebase to safety-adjacent claims) accept shipping a
   primitive with a known, unfixable deadlock class, or does `Stop`/
   `Cont` stay `Unsupported` on Windows permanently (a registered
   divergence, not a gap to close)?
5. **Does a from-scratch pty line discipline replace ConPTY, or layer
   on top of it?** `platform::pty` (D13) is ConPTY-based and already
   landed with real consumers implied (`docs/design-discussion-pty.md`).
   Replacing it with a Cygwin-style named-pipe pty would be a second,
   parallel PTY backend, not an extension of the first — a much bigger
   commitment than it sounds, and one that duplicates rather than
   builds on #82/#83's landed work. A shim that intercepts ConPTY's
   VT input stream to add ICANON/ISIG-style signal-generating-character
   emulation *without* replacing the pty transport itself is a smaller,
   different option worth naming separately rather than conflating with
   "build MSYS2's pty."

## What this document does not decide

Whether to build any of subsystems 1–4, in what shape, on what
timeline, or whether "msys parity" is even a goal worth pursuing given
the participation boundary above (a rustils-only pgid/signal model
helps rustils-spawned trees, not arbitrary Windows processes, the same
ceiling MSYS2 itself has always had). That is the RFC-level call the
owner makes before any of this becomes code — this is the input to that
call, not the call itself.
