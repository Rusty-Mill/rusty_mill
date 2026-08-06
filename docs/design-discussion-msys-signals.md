# Design discussion — arbitrary Windows signal delivery (msys-parity subsystem 2, deeper scope)

Not a decision record. `docs/design-discussion-msys-parity.md` scoped
four subsystems msys/Cygwin builds to get POSIX-shaped job control and
signals on Windows; its own open question 2 flagged one of the four —
richer `Signal` delivery, *without* the full pgid/session table (§1) or
job-control terminal handoff (§3) — as separable and plausibly valuable
on its own. This document takes that one subsystem to implementation-
shaped depth, the same way `docs/design-discussion-pty.md` did for D13
before it landed. Still not a build — no named consumer exists, and
§9's open questions still need an owner call.

## Where this sits against what's already real

Three things already exist and this slice would extend, not replace:

- **`platform::process::Signal`** (`crates/platform/src/process.rs:216`)
  — `Term`/`Int`/`Hup`/`Quit`/`Kill`/`Stop`/`Cont`, the *sending* side's
  vocabulary. `Child::kill_tree`/`kill_single` already take one; Windows
  refuses everything but `Kill` today (divergence 008) — verified at
  all three call sites: `WindowsChild::kill_tree`/`kill_single`
  (`platform-windows/src/process.rs:79,99`) and
  `WindowsGroupHandle::kill_tree`/`kill_single` (same file, `:174,186`),
  each an identical `if sig != Signal::Kill { return
  Err(Unsupported) }` guard before falling through to
  `TerminateJobObject`/`TerminateProcess`.
- **`platform::events::{SignalEvent, SignalSource}`**
  (`crates/platform/src/events.rs`) — the *receiving* side, and
  deliberately narrow by design: `SignalEvent` has exactly three
  variants (`Interrupt`/`Terminate`/`Hangup`), and the module doc says
  why — "the full unix signal zoo is policy territory and stays with
  consumers." This is a receiver-observes-what-changed-my-own-control-
  flow surface, not a general IPC channel.
- **The single-atomic-slot mechanism** (`platform-windows/src/sys/
  csignals.rs`) — `SetConsoleCtrlHandler`'s callback does exactly one
  `AtomicU32::store`; `take()` is `swap(NONE)`. This is the D6
  discipline ("one atomic store; consumption at safe points") this
  slice's own delivery mechanism should match, not reinvent.

## The one fact that changes everything relative to Cygwin

Cygwin's signal machinery (`sigproc.cc`, cited in the parent document)
works because `cygwin1.dll` is **loaded into and initializes inside
every Cygwin-linked process automatically**, before that process's own
`main()` runs — the signal-listening thread starts itself as a
side effect of the DLL attaching. rustils has no equivalent injection
mechanism and isn't trying to build one (a DLL that auto-attaches to
arbitrary processes is a wholly different, much larger, and much more
invasive kind of project). **A process can only ever receive a signal
through this mechanism if its own code explicitly started listening
first** — there is no way to `kill()` an arbitrary already-running
Windows process the way real POSIX `kill(2)` can address any pid the
caller has permission for. This is not a limitation to work around; it
is the honest ceiling of what an opt-in library can offer, and it must
be the first sentence of any doc comment this slice ships, not a
footnote.

Concretely: this only ever helps a tree of cooperating,
`platform`-aware processes (the same participation boundary the parent
document already named for the whole msys-parity effort) — most
usefully, a parent that spawned its children through `Spawner::spawn`
and has each child call the new listener-install function near the top
of its own `main`.

## Proposed mechanism

### Naming — no shared table needed for this slice

A named pipe per listening process, named deterministically from its
own OS pid: `\\.\pipe\rustils-signal-<pid>`. A sender that already has
a `Child`/`GroupHandle` has `id()` (the real pid) in hand — **this
slice needs none of subsystem 1's shared pgid/session table** to
deliver point-to-point. That table only becomes necessary if a future
slice wants `killpg`-style broadcast-by-group; deliberately out of
scope here, and worth recording as a real scope reduction against the
parent document's four-subsystems-as-one-unit framing.

### Listener side (new)

A background thread a process starts explicitly — proposed shape:

```rust
// crates/platform/src/events.rs — a new, separate trait, not a
// widening of SignalEvent/SignalSource (see "Why a separate surface"
// below).
pub enum PeerSignal { Term, Int, Hup, Quit, Stop, Cont } // no Kill —
// Kill is unconditionally TerminateProcess-equivalent already; nothing
// to "deliver" to a listener that has no chance to observe it.

pub trait PeerSignalSource {
    fn install(&self) -> Result<()>; // idempotent, like SignalSource
    fn take(&self) -> Option<PeerSignal>; // same swap-and-clear shape
}
```

`WindowsPeerSignalSource::install` creates the named pipe
(`CreateNamedPipe`, `PIPE_ACCESS_DUPLEX`, one instance is enough for a
single-slot-coalescing design — a second sender while one delivery is
still unconsumed just coalesces, matching `csignals.rs`'s own
"a burst since the last call coalesces to the most recent event"
contract) and spawns a thread looping `ConnectNamedPipe` →
`ReadFile` (one `u32` signal id) → `PENDING.store` → disconnect → loop.

### Wire format

One `u32`, no payload — smaller than Cygwin's own `sigpacket` (which
carries a full `siginfo_t`) because this slice doesn't attempt sender
identity, queueing, or blocking/masking semantics; it delivers exactly
as much as `SignalEvent`'s own existing minimalism already models.

### Sender side (extends existing code, not new code)

The three `if sig != Signal::Kill { Unsupported }` guards cited above
become: for `Stop`/`Cont`/`Term`/`Int`/`Hup`/`Quit`, `CreateFileW` the
target's pipe name and write the packet; `Kill` keeps its existing
`TerminateJobObject`/`TerminateProcess` path unchanged (there is
nothing to "deliver" — the process ends unconditionally, matching
Cygwin's own `SIGKILL` handling, which is real-terminate, not
signal-queue delivery, even in their model).

### Why a separate surface, not a wider `SignalEvent`

`SignalEvent`'s own module doc draws the line deliberately narrow:
identities a process's *own environment* can generate against it
(console control events on Windows, real signals on Linux) — three
variants, on purpose, "the full unix signal zoo... stays with
consumers." Cramming `Quit`/`Stop`/`Cont` into that enum just because a
second, unrelated delivery channel now exists would blur a boundary
D6 drew for a reason, and would make every existing `SignalSource`
consumer's `match` on `SignalEvent` non-exhaustive for events that can
now arrive from a completely different mechanism (named-pipe delivery,
gated on this process having opted in) than the one `SignalEvent`
was built to describe (console-ctrl events, always live for any
console process). A parallel, explicitly-named `PeerSignalSource` keeps
both contracts honest: a consumer that only cares about "did my
console tell me to stop" doesn't have to reason about pipes at all.

## Security: the default named-pipe ACL is a real footnote to close, not skip

`CreateNamedPipe` with a null `SECURITY_ATTRIBUTES` uses a default DACL
that — undocumented-but-real Windows behavior worth stating plainly —
grants connect access to any local, authenticated user, not just the
pipe owner. Left unaddressed, this would let any other local account
send fabricated "signals" into a `platform`-aware process. This crate
already has the exact precedent for the fix: `platform::net`'s Unix
listener bind restricts to mode `0600` (D16) specifically to stop
other local users from connecting to an otherwise-ambient-namespace
IPC endpoint. The Windows equivalent is an explicit
`SECURITY_DESCRIPTOR` naming only the creating user's SID, built and
attached the same way `sys::security`'s existing Windows admissions
already construct SIDs/ACLs for other slices (`platform::security`) —
not a new primitive, an application of one already in this codebase.

## Failure modes — a three-way distinction Unix `kill(2)` doesn't have

`kill(pid, sig)` on Unix has two outcomes: delivered, or `ESRCH` (no
such process). This mechanism has **three**: the target process
doesn't exist at all; the target exists but never called `install()`
(no pipe, `CreateFileW` fails `ERROR_FILE_NOT_FOUND`); or the target
exists and is listening (delivered). The first two are
indistinguishable from the pipe-open failure alone — `ERROR_FILE_NOT_FOUND`
either way — and telling them apart would need a second, separate
`OpenProcess`-based existence check (the same call `Spawner::adopt`
already uses). Whether a caller needs that distinction, or whether
"pipe open failed" collapsing both into one `ErrorKind` (`NotFound`?
a new one?) is honest enough, is an open question below — not resolved
here.

## A real advantage over the Unix precedent: no stale-cleanup class

Named pipes are destroyed automatically when their last server handle
closes — unlike a Unix domain socket, which can leave a stale path
behind after an ungraceful exit (the exact problem `platform::net`'s
own stale-cleanup-bind logic exists to solve, D16). A crashed listener
here just makes the next sender's `CreateFileW` fail cleanly
(`ERROR_FILE_NOT_FOUND`) with nothing left to clean up — genuinely
simpler than the Unix side of this crate's own precedent, not merely
different.

## `Stop`/`Cont`: notify, don't suspend — a deliberate divergence from Cygwin's own mechanism

The parent document's subsystem 3 (`SuspendThread`/`ResumeThread`) is
the *effect* of `SIGSTOP`/`SIGCONT` — actually pausing every thread in
the target. This slice's own wire format can carry the *notification*
(`PeerSignal::Stop`/`Cont` arriving via `take()`) without rustils ever
calling `SuspendThread` on anyone else's threads itself. A receiving
process's own code decides how to honor a `Stop` notification — most
plausibly blocking on a condvar until a matching `Cont` arrives,
entirely in its own userspace, no cross-process thread suspension at
all. This sidesteps subsystem 3's own flagged hazard (`SuspendThread`
on a thread mid-lock is a real, unfixable deadlock class) at the cost
of requiring the target's *cooperation* to actually stop. Worth its own
explicit owner call, separate from whether this whole slice happens at
all — taken further in `docs/design-discussion-msys-stop-cont.md`.

**Correction** (that document's own finding): Cygwin's real `SIGSTOP`
handler suspends *sibling threads in the same process*, from a thread
already running inside the target — cooperative in exactly the sense
this section proposes, not an outside process reaching in. The real
design fork isn't "notify vs. Cygwin's external approach"; it's
"notify only" vs. "notify, then actually suspend siblings the same way
Cygwin does" — both in-process, both requiring the target to have
opted in by running this crate's listener at all. See the linked
document for the full mechanism and its own open hazards.

## Rough size, if it happens

New: `platform::events::{PeerSignal, PeerSignalSource}` (trait +
enum, `platform/src/events.rs`); `platform-windows`'s
`WindowsPeerSignalSource` + a `sys::peer_signal` module (named-pipe
listener thread, SID-scoped ACL construction, wire encode/decode) —
comparable in shape and size to `sys::csignals.rs` plus a security-
descriptor helper, not a small addition but not `sys::pty`-sized
either. Changed: the three `Signal::Kill`-only guards in
`platform-windows/src/process.rs` gain a non-`Kill` arm. No Linux
changes — Linux already has real `kill`/`killpg` (D1), this slice is
Windows-only by construction.

## Open questions for the owner, not decided here

1. **Still no named consumer.** This document makes the *shape*
   concrete; it does not manufacture a reason to build it. The parent
   document's question 1 still applies.
2. **`PeerSignalSource` as a new trait, or folded into `SignalSource`
   with a wider `SignalEvent`?** This document argues for separate
   (see "Why a separate surface" above) but that's a real design call,
   not a foregone one — a consumer that wants one unified `take()` for
   "any reason I might need to react" would find two traits more
   ceremony than one wider enum.
3. **The three-way failure-mode question above** — collapse
   "nonexistent" and "exists but not listening" into one `ErrorKind`,
   or pay for a second `OpenProcess` check to distinguish them? No
   consumer exists yet to say which one it needs.
4. **Notify-only `Stop`/`Cont` (cooperative) vs. deferring them
   entirely until/unless subsystem 3's `SuspendThread` question is
   separately resolved** — shipping `PeerSignal::Stop`/`Cont` as
   notify-only is weaker than real job control but real (as opposed to
   `Unsupported`) and safe; shipping only `Term`/`Int`/`Hup`/`Quit` and
   leaving `Stop`/`Cont` refused is simpler and matches this crate's
   general discipline of not offering a capability until its full
   semantics are settled. Not resolved here.
5. **Does a listener process need to uninstall/stop the thread
   cleanly, or does process exit (which destroys the pipe
   automatically, per the advantage noted above) make that
   unnecessary?** Leaning toward "unnecessary — let process exit do
   it," but worth stating explicitly rather than assuming, since every
   other install-a-background-thread precedent in this codebase
   (`sys::pty::spawn_exit_watcher`) does have an explicit teardown
   path for a different reason (avoiding a `ClosePseudoConsole`
   deadlock) that doesn't obviously apply here.

## What this document does not decide

Whether to build this slice, whether `Stop`/`Cont` ship at all in a
first version, or whether `PeerSignalSource` is the right trait shape
versus widening `SignalEvent`. Same posture as the parent document:
input to an owner's RFC-level call, not the call itself.
