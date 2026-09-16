# ADR-0002: signalfd as the primary recommended path for event-driven signals

Status: Accepted
Date: 2026-07-23

## Context

REVIEW.md Round 3 §L (and [issue #33](https://github.com/baileyrd/rusty_libc/issues/33))
raised a design question rather than a missing primitive: this crate's signal
story — `sigaction`/`signal`, `sigprocmask`, `sigpending`, `sigsuspend` — is
complete and correct, but it is also the most failure-prone part of anything
built on it. A meaningful fraction of this crate's hardest work exists purely
to make the *handler* path correct: the x86_64 `sa_restorer` trampoline
(`src/signal.rs`'s whole "x86_64 restorer problem" section), the
signal-storm stress test, and the general discipline of keeping handler
bodies async-signal-safe. None of that difficulty is intrinsic to "react to a
signal" — it's intrinsic to "run arbitrary code from inside a real signal
handler," which is a much harder problem than the shell actually has.

`rush` (this crate's consumer) needs to react to `SIGCHLD` (a child changed
state), `SIGINT`/`SIGQUIT` (user interrupt), `SIGWINCH` (terminal resize),
`SIGTERM`/`SIGHUP` (shell should exit/hang up), and — to implement the POSIX
`trap` builtin — essentially arbitrary signals a user script asks to catch.
In every one of these cases, the actual work to do in response (print a
message, reap a child, redraw a prompt, run a user's trap body, unwind
interpreter state) is not async-signal-safe code by any stretch: it allocates,
it calls into the interpreter, it may itself call other syscalls freely. A
real handler is the wrong tool for that; the entire point of a handler is to
run *before* such code is safe to run at all.

`signalfd(2)` sidesteps this: block the signals of interest with
`sigprocmask`, open a `signalfd` for that mask, and the fd becomes readable
(with a `poll`-compatible event) once a signal is pending, yielding a
`struct signalfd_siginfo` describing it on `read`. There is no handler, no
restorer, no async-signal-safety constraint on what runs next — "next" is
just the next iteration of the shell's own `poll` loop, with the full run
time environment available. This also folds signal delivery into the exact
same readiness model the crate already uses for I/O (`fd::poll`), rather than
running it as a structurally different, handler-based side channel.

## Decision

**`signalfd` becomes the primary recommended path for the signals a
job-control shell needs to react to in its own control flow** — the full set
above, including `trap`-style user-installed handlers (the shell blocks the
signal and dispatches the trap body from its main loop on a `signalfd` read
event, rather than installing a real handler for it). `sigaction`/`signal`
are kept, unchanged, as:

1. **Required plumbing.** `signalfd` still needs `sigprocmask` to block the
   signals it watches — this crate isn't deprecating `sigprocmask`, only
   de-emphasizing *handler installation* (`sigaction`) for the cases above.
2. **The correct tool for synchronous/hardware-fault signals** — `SIGSEGV`,
   `SIGBUS`, `SIGFPE`, `SIGILL`, `SIGTRAP`. These fire mid-instruction, in
   the faulting context itself; blocking-and-reading-later doesn't compose
   with "the faulting instruction cannot simply be resumed as if nothing
   happened." A shell that wants to report a child's crash observes it via
   `wait`'s `WIFSIGNALED`/`WTERMSIG`, not by catching these in itself, so
   this exception is largely moot for the actual consumer, but it means
   `sigaction` is not being deprecated as a primitive — only as the default
   recommendation for the shell's own asynchronous signal set.
3. **Available for any consumer that needs classic handler compatibility**
   for reasons outside this crate's control (e.g. embedding code that
   installs its own handlers and expects real signal-delivery semantics).

`SIGKILL`/`SIGSTOP` are uncatchable by either mechanism and are unaffected by
this decision.

One second-order benefit worth naming: signals a consumer handles
exclusively via `signalfd` are, definitionally, blocked at the process/thread
level. A blocked signal cannot interrupt a blocking syscall, so `SA_RESTART`
reasoning (and `EINTR` handling generally) simply does not apply to signals
routed this way — one whole category of "did I remember to handle `EINTR`
here" bugs disappears for the signal set that goes through `signalfd`.

## Alternatives considered

- **Keep `sigaction`/handlers as primary, `signalfd` unimplemented.** Rejected:
  this is the status quo the issue is questioning, and it means every new
  signal a consumer wants to react to needs handler-safety review (can this
  handler body allocate? call into locked state? etc.) for work that is,
  after the fact, always just "wake up the main loop." That's a worse
  default for a shell interpreter, whose signal-reactive code is
  characteristically *not* async-signal-safe.
- **Replace `sigaction` entirely.** Rejected: synchronous fault signals
  (§Decision, point 2) don't fit the block-and-poll model at all, and some
  embedding consumers may have their own reasons to want real handlers.
  Removing a correct, already-hardened primitive to enforce a stylistic
  preference isn't worth it — this crate keeps both, sized to what each is
  actually good at.
- **Leave the decision unstated and let each consumer choose ad hoc.**
  Rejected: the entire point of REVIEW.md §L was that this is exactly the
  kind of choice a consumer shouldn't have to rediscover independently —
  `rush` (and any future consumer) benefits from `rusty_libc` having an
  opinion here, documented once.

## Consequences

- `signalfd(2)`/`signalfd4(2)` is implemented (`signal::signalfd`,
  `SFD_NONBLOCK`/`SFD_CLOEXEC`, `SignalfdSiginfo` matching the kernel
  layout), tracked in #59 and shipped separately from this decision, per
  #33's own scoping.
- Existing `sigaction`/`sigprocmask`/`sigsuspend` code, tests, and the
  x86_64 restorer machinery are unaffected — nothing is being removed or
  deprecated at the API level, only re-pointed as the *secondary* path for
  new consumers reacting to asynchronous signals.
- README/module docs for `signal` should, once `signalfd` ships, note the
  recommendation so a new consumer reads it before reaching for `sigaction`
  out of habit.
