# Design discussion — real Stop/Cont delivery (msys-parity subsystem 3, deeper scope)

Not a decision record. `docs/design-discussion-msys-parity.md` named
subsystem 3 as `SuspendThread`/`ResumeThread`-based `SIGSTOP`/`SIGCONT`
emulation, flagged its known deadlock hazard, and left the choice
between it and a notify-only design as an explicit open question.
`docs/design-discussion-msys-signals.md` (subsystem 2, merged) then
proposed notify-only `Stop`/`Cont` as the safer default and deferred the
real-suspend question to "its own explicit owner call, separate from
whether this whole slice happens at all." This document is that call's
input: re-reading Cygwin's actual `SIGSTOP` implementation (not the
one-line summary the parent document worked from) turns up a materially
different, safer mechanism than "an external process reaches into a
foreign process and suspends its threads" — worth correcting before an
owner decides anything.

## Correcting the parent document's framing

Cygwin's `sig_handle_tty_stop` (`winsup/cygwin/exceptions.cc:885`) is
**not** invoked by some other process reaching in. It runs *inside the
target process itself*, on whichever thread Cygwin's own in-process
signal dispatch was already using to handle the incoming `SIGSTOP` —
the same dispatch path both an external `kill(pid, SIGSTOP)` and an
internally-generated Ctrl-Z go through by the time a handler runs. That
thread then:

```cc
pthread::suspend_all_except_self ();                  // exceptions.cc:902
DWORD res = cygwait (NULL, cw_infinite, cw_sig_cont);  // exceptions.cc:903 — blocks itself, cooperatively
pthread::resume_all ();                                // exceptions.cc:904
```

`suspend_all_except_self`/`resume_all`
(`local_includes/thread.h:462-470`) walk Cygwin's own tracked-thread
list and call plain `SuspendThread`/`ResumeThread`
(`thread.cc:1105-1116`) on each — but every one of those threads is a
**sibling in the same process**, and the thread doing the suspending
never suspends itself; it blocks on an ordinary wait instead. This is
exactly the *notify-only, cooperative* shape
`design-discussion-msys-signals.md` already proposed, not the riskier
*external* suspend the parent document's one-line summary implied. The
practical upshot: **subsystem 3, done Cygwin's own way, is a natural
extension of subsystem 2's already-scoped `PeerSignalSource` listener
thread** — no new cross-process capability, no `PROCESS_SUSPEND_RESUME`
handle to a foreign process, nothing beyond what a thread can already do
to its own siblings.

## A real, documented gap in Cygwin's own mechanism — worth citing honestly

`suspend_all_except_self`'s own comment doesn't hide this:

> `/* FIXME! This does nothing to suspend anything other than the main
> thread. */` — `exceptions.cc:897`

`pthread::threads` is Cygwin's *own* intrusive list of threads created
through *its own* `pthread_create` wrapper — a thread spawned via a raw
`CreateThread`, or one belonging to a linked non-Cygwin library, is
invisible to it and is never suspended. This is not a hypothetical
concern this document is inventing; it's admitted, in-source, in a
25-year-old, widely-deployed implementation. A rustils port would face
the identical question and has no equivalent tracked-thread registry of
its own to reuse or omit — see the enumeration options below.

## Two ways to actually enumerate and suspend "every thread," neither free

1. **`CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0)` +
   `Thread32First`/`Thread32Next` filtered by `th32OwnerProcessID`,
   `OpenThread(THREAD_SUSPEND_RESUME, ...)` + `SuspendThread` per
   thread.** Documented, public, already the kind of primitive this
   crate uses elsewhere (`windows-sys`'s ordinary `Win32_*` surface —
   no new workspace Cargo feature beyond what a `Win32_System_Diagnostics_ToolHelp`
   admission would need). Catches *every* thread in the process
   regardless of how it was created — strictly better coverage than
   Cygwin's own registry-based approach in this one respect. **Not**
   race-free: a thread created between the snapshot and the suspend
   loop's last iteration escapes suspension entirely, a different hole
   than Cygwin's "wrong list" hole but the same class of gap.
2. **`NtSuspendProcess`/`NtResumeProcess`** — undocumented but
   extremely stable `ntdll.dll` exports (in continuous use since Vista
   by Task Manager's own "Suspend" action, Process Explorer, and other
   widely-shipped tools), suspending the *entire process* atomically in
   the kernel in one call — no enumeration, no race window, no missed
   thread. **Not present in `windows-sys` 0.59's covered surface at
   all** (verified: no `NtSuspendProcess`/`NtResumeProcess` anywhere
   under its `Wdk`/`Win32` trees) — using it would mean a hand-rolled
   `unsafe extern "system"` binding against an undocumented symbol,
   the same kind of escalation Track P's raw Linux syscalls represent
   (D-1/D4), with the attendant risk undocumented APIs carry: no
   contractual guarantee across Windows versions, however stable it has
   been in practice.

**This is a real tradeoff, not a foregone pick**: option 1 is
documented and race-prone; option 2 is race-free and undocumented. This
document does not resolve it — see the open questions below.

## Where this plugs into subsystem 2's already-scoped listener

`design-discussion-msys-signals.md` proposed a single listener thread
per opted-in process, blocking on `ConnectNamedPipe`/`ReadFile` in a
loop. Acting on `PeerSignal::Stop`/`Cont` for real (rather than just
setting the atomic slot `PeerSignalSource::take()` later reads) needs
a small state machine in that same thread, mirroring Cygwin's own
"the dispatching thread is the one that blocks" shape — not a second
thread, unless robustness against a stuck read argues for one later:

- Normal state: blocked in `ReadFile` on the pipe.
- On `Stop`: suspend every other thread (option 1 or 2 above), then
  block on a **second**, distinct wait — not the same `ReadFile` (the
  pipe needs draining by *something* while stopped, or a sender's next
  `Cont` write blocks against a full pipe buffer) — most simply, a
  dedicated Win32 event this thread waits on, set by continuing to read
  the pipe from... itself, which is a contradiction. The clean
  resolution, matching Cygwin's structure directly: **don't suspend the
  listener thread's own ability to read the pipe at all** — after
  suspending every *other* thread, loop back into the exact same
  `ReadFile` call, just with the process now stopped in every thread
  but this one. A `Cont` packet arriving is read normally, triggers
  `ResumeThread` on everyone, and the loop returns to its ordinary
  state. This is simpler than it first looks specifically because
  Cygwin already proved the "one thread stays live to hear the resume"
  shape is sufficient — no second thread needed for a first version.

## The half this document found that subsystem 2 alone can't cover: observability

`Child::wait_job`/`try_wait_job` (`platform/src/process.rs:283,292`)
and `ExitStatus::Stopped(i32)`/`Continued`
(`platform/src/process.rs:194,198`) already exist — landed for Linux
(D10), explicitly `Unsupported` on Windows today
(`platform-windows/src/process.rs`'s own comment: *"No stop/continue
analog on Windows (D8): job-control suspend is a Unix-only concept"*).
A real Windows `Stop`/`Cont` mechanism is only useful to a shell-style
consumer if `wait_job` can observe it — and this is where Cygwin's
*second*, separate mechanism matters: `_pinfo::alert_parent`
(`pinfo.cc:1352`) writes a single raw `char` (the signal number) to
`my_wr_proc_pipe` — **a dedicated pipe established between parent and
child at spawn time**, read by the parent's own `proc_waiter` background
thread (`pinfo.cc:1336`) — not the general arbitrary-pid signal pipe
subsystem 2 built. Cygwin itself keeps these as two different
mechanisms, and the reason generalizes: **only a process that controls
the child's own `CreateProcessW` call can wire up a status pipe the
child inherits automatically** — subsystem 2's named pipe (keyed by a
pid the sender merely knows) has no equivalent "this is my parent"
relationship to piggyback on.

**Consequence, worth stating plainly**: `wait_job`/`try_wait_job`
observability can only ever work for **direct children spawned through
this crate's own `Spawner`** — never for an arbitrary cooperating pid
reached only through subsystem 2's delivery mechanism. This is not a
temporary gap to close later; it is a structural fact about which
relationship each mechanism can express, the same way `GroupSpec::JoinGroup`
is `Unsupported` on Windows for a *different* structural reason
(divergence 008). Concretely, this crate already has the right-shaped
precedent to reuse: `Stdio::Pipe` + the `STARTF_USESTDHANDLES`
inheritable-handle wiring decided in `docs/extraction-map.md`'s
suggested-sequence step 4 (*"per-spawn `STARTF_USESTDHANDLES`... only
this spawn's handles are inheritable"*) — a fourth inheritable pipe,
wired the identical way stdin/stdout/stderr already are, dedicated to
one-byte stop/cont/exit notifications from child to parent, read by a
background thread on the parent's `WindowsChild` that
`wait_job`/`try_wait_job` consult instead of returning `Unsupported`.

## Security

No new surface beyond what subsystem 2 already has to solve. Suspension
itself is purely in-process once a `Stop` packet arrives — there is
nothing here for another local user to abuse that subsystem 2's own
SID-scoped named-pipe ACL doesn't already have to guard. The new
parent-child status pipe (previous section) is spawn-time-inherited,
the same trust boundary `Stdio::Pipe` already operates under — no
ambient name for an unrelated process to discover or connect to at all.

## Hazards, stated once, not softened

`SuspendThread` on a thread mid-lock-acquisition (loader lock, heap
lock, CRT lock) can hang the *entire* process the instant anything else
tries to touch that lock — real, documented, and, per the parent
document's own open question 4, not something this codebase's existing
"needs live verification, not just inspection" bar has ever accepted
for a *known, unfixable* deadlock class elsewhere. Doing the suspending
in-process (this document's finding) removes the cross-process-attack
framing but does **not** remove this specific hazard — a sibling thread
can still be mid-lock when `Stop` arrives, exactly as it can in Cygwin's
own mechanism today. Whether that's an acceptable, Cygwin-precedented
risk or a hard blocker is squarely an owner call, not a technical
question this document can resolve by more research.

## Rough size, if it happens

Extends, doesn't replace, subsystem 2's `WindowsPeerSignalSource`:
the suspend/resume state machine inside its existing listener thread,
plus (option 1) a `Win32_System_Diagnostics_ToolHelp` admission and
enumeration helper, or (option 2) a hand-rolled `ntdll` binding behind
its own documented-undocumented-API admission, whichever the owner
picks. Separately: a new inheritable status pipe in
`platform-windows/src/process.rs`'s spawn path (`WindowsChild`'s
existing `pipes: [Option<OwnedWinHandle>; 3]` becomes a 4th slot, or a
dedicated field) plus a background reader thread, and real
`wait_job`/`try_wait_job` bodies replacing today's `Unsupported`
returns. Comparable in shape to subsystem 2 itself, not smaller.

## Open questions for the owner, not decided here

1. **Still no named consumer** — same as every open item in this
   family. This document sharpens the shape; it doesn't manufacture a
   reason to build it.
2. **Toolhelp32 enumeration (documented, race-prone) vs.
   `NtSuspendProcess`/`NtResumeProcess` (undocumented, atomic)?** A real
   tradeoff between this codebase's usual "stick to the documented
   `windows-sys` floor" discipline (D-1) and a materially safer
   mechanism that sits outside it.
3. **Does the in-process, cooperative-blocking hazard above (a sibling
   thread suspended mid-lock) meet this codebase's bar for shipping a
   capability at all**, given the parent document's own precedent of
   treating "known, unfixable deadlock class" as disqualifying
   elsewhere — or is "Cygwin has shipped this exact mechanism for
   decades without it being a practical blocker" sufficient precedent
   to accept the same risk here?
4. **Is `wait_job`/`try_wait_job` observability (the direct-child-only
   status pipe) in scope for the same landing as `Stop`/`Cont`
   delivery, or does `Stop`/`Cont` ship first as fire-and-forget** (a
   receiving process can act on it, but a sender/parent has no portable
   way to confirm it happened) **with observability as a later,
   separately-gated slice?** Mirrors subsystem 2's own question 2 about
   splitting a landing along its natural seams rather than shipping
   everything as one unit.

## What this document does not decide

Whether to build this, which suspend mechanism to use, or whether the
observability half ships alongside or after. Same posture as every
other document in this family: input to an owner's call, not the call
itself.
