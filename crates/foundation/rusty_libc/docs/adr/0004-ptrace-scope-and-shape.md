# ADR-0004: `ptrace` scope and API shape

Status: Proposed
Date: 2026-07-23

## Context

Issue #102 (`process_vm_readv`/`process_vm_writev` + `ptrace`), filed from the
parity-loop's `cargo public-api` diff against `libc` 0.2.189, originally
bundled both `libc::ptrace` symbols. `process_vm_readv`/`process_vm_writev`
shipped on their own (no `ptrace` attach needed for those two); `ptrace` was
split out into this issue (#118) because, unlike every other syscall this
crate wraps, its argument meaning is entirely request-dependent (sometimes an
address, sometimes a data word, sometimes an out-pointer) and its useful
forms need per-arch register-layout structs verified against raw kernel
source — real work, not a thin 1:1 wrapper, and the kind of decision this
crate's own precedent (ADR-0002, ADR-0003) settles via a written proposal
before an implementation PR rather than mid-PR.

As with sockets before this (ADR-0003), no confirmed `rush` consumer need
exists: `rush` is a job-control shell, not a debugger, and nothing in its
own tree calls or wraps `ptrace` today. Round 4's "no known consumer need"
bar would ordinarily decline this outright, the same as `flock`/`chroot`/
`sendfile`. This ADR exists because the user chose to work through the
scoping question rather than skip it — the same kind of explicit
build-ahead-of-need call as ADR-0003, not an automatic yes.

An earlier draft of this ADR recommended excluding `PTRACE_SYSCALL`-based
syscall tracing from the first slice, on the grounds that it needs a
`wait4`-driven stop-disambiguation loop and per-arch syscall-register
conventions layered on top of `GpRegs` — real additional work, not a
one-line addition alongside `PTRACE_CONT`. That recommendation was
overridden: the decision is to include it now rather than defer it to a
future issue.

## Decision

**Add a `ptrace` module, scoped to a tracer/tracee lifecycle, portable
register access, and syscall-entry/exit tracing — explicitly excluding only
memory peek/poke (superseded by #102) and the legacy `PTRACE_ATTACH`/
`GETREGS` forms superseded by their modern equivalents.**

1. **New standalone `src/ptrace.rs` module**, not folded into `process` —
   matching the one-file-per-primitive-family pattern already used for
   `mmap`/`socket`/`epoll`/`eventfd`/`inotify`/`sysinfo`. `process.rs` is
   already this crate's largest file; a tracee-lifecycle-plus-register-access
   concern is distinct enough to earn its own module rather than grow it
   further.

2. **One function per request, not an enum dispatcher** — matching every
   other module's convention (`fd`, `fs`, `process`, `socket`: one function
   per verb, never a single dispatch call taking an operation enum):
   - `ptrace_traceme() -> Result<(), Errno>` — child-side, called before
     `execve`, so the parent automatically becomes tracer at exec.
   - `ptrace_seize(pid: i32, options: i32) -> Result<(), Errno>` —
     `PTRACE_SEIZE`, the modern attach that does *not* group-stop the
     tracee (unlike the legacy `PTRACE_ATTACH`, which this omits: `SEIZE`
     strictly supersedes it for any tracer written against this crate).
   - `ptrace_cont(pid: i32, signal: i32) -> Result<(), Errno>` —
     `PTRACE_CONT`, resume a stopped tracee, optionally re-injecting
     `signal` (`0` for none).
   - `ptrace_singlestep(pid: i32, signal: i32) -> Result<(), Errno>` —
     `PTRACE_SINGLESTEP`, same shape as `ptrace_cont`.
   - `ptrace_detach(pid: i32, signal: i32) -> Result<(), Errno>` —
     `PTRACE_DETACH`, resume and stop tracing.
   - `ptrace_setoptions(pid: i32, options: i32) -> Result<(), Errno>` —
     `PTRACE_SETOPTIONS`; primarily so a tracer can set
     `PTRACE_O_EXITKILL` (tracee dies if the tracer does) rather than
     leaving an orphaned, permanently-stopped process behind on a crash.
   - `ptrace_getregset(pid: i32) -> Result<GpRegs, Errno>` /
     `ptrace_setregset(pid: i32, regs: &GpRegs) -> Result<(), Errno>` —
     `PTRACE_GETREGSET`/`PTRACE_SETREGSET` with `NT_PRSTATUS`, **not** the
     legacy arch-specific `PTRACE_GETREGS`/`SETREGS`: `GETREGSET`'s calling
     convention (an `iovec` wrapping the arch buffer) is uniform across
     arches even though the register-set layout inside it isn't, which
     keeps the per-arch divergence confined to `GpRegs` itself rather than
     leaking into the call shape too.
   - Killing a tracee is just `process::kill(pid, SIGKILL)` — no dedicated
     wrapper; `PTRACE_KILL` is itself deprecated by the kernel in favor of
     exactly this.
   - `ptrace_syscall(pid: i32, signal: i32) -> Result<(), Errno>` —
     `PTRACE_SYSCALL`, same shape as `ptrace_cont`/`ptrace_singlestep` but
     resumes the tracee only until the next syscall-entry-or-exit boundary
     rather than running free. This is the primitive an `strace`-style
     tracer is built on.
   - `is_syscall_stop(status: i32) -> bool` — a small convenience predicate
     layered on the existing `wait::wifstopped`/`wait::wstopsig` (no new
     `wait` module additions needed): with `PTRACE_O_TRACESYSGOOD` set via
     `ptrace_setoptions` (required for this to be reliable — see below),
     `wait::wstopsig(status) == SIGTRAP | 0x80` identifies a syscall-stop,
     as opposed to a plain `SIGTRAP` the tracee actually raised itself.
     Without `PTRACE_O_TRACESYSGOOD`, syscall-stops and real `SIGTRAP`
     delivery are **indistinguishable** — this predicate's doc says so
     plainly, and `ptrace_syscall`'s doc points at `ptrace_setoptions`
     first.
   - Reading which syscall (and its arguments, or return value) is in
     flight at a syscall-stop reuses `GpRegs`' existing fields — no new
     struct: x86_64's `orig_rax` (a field `struct user_regs_struct` already
     carries specifically because `rax` itself flips from "syscall number"
     at entry to "return value" at exit) always holds the syscall number
     regardless of entry/exit; aarch64's `x8` never gets overwritten by the
     return value (which lands in `x0`), so it needs no `orig_x8`
     equivalent. Distinguishing an *entry* stop (arguments valid) from an
     *exit* stop (return value valid) is **not** encoded in the stop event
     itself on either arch — `PTRACE_SYSCALL` stops strictly alternate
     entry/exit/entry/exit, so this is left as the caller's own state to
     track (a single toggled `bool`), documented plainly rather than
     papered over with an implicit crate-side state machine.

3. **`GpRegs`: two `#[cfg(target_arch = ...)]`-gated struct definitions**,
   mirroring `epoll::EpollEvent`'s precedent for genuine per-arch kernel
   layout divergence — `struct user_regs_struct` (x86_64,
   `arch/x86/include/asm/user_64.h`) and `struct user_pt_regs` (aarch64,
   `arch/arm64/include/uapi/asm/ptrace.h`), each verified against raw
   kernel source (not summarized) with its own `size_of`/`offset_of` const
   assertions, each exposing the fields a tracer actually needs (instruction
   pointer, stack pointer, general-purpose registers) rather than every
   field either struct happens to define.

4. **Explicitly out of scope:**
   - **`PTRACE_PEEKTEXT`/`PEEKDATA`/`POKETEXT`/`POKEDATA`** — the legacy
     word-at-a-time memory access `ptrace` itself provides is strictly
     subsumed by `process::process_vm_readv`/`process_vm_writev` (#102,
     already shipped), which read/write arbitrary-length ranges in one call
     without an attach. Adding the `ptrace`-based word-at-a-time forms would
     be pure duplication with a worse interface.
   - **`PTRACE_ATTACH`** — superseded by `PTRACE_SEIZE` (see above); no
     reason to carry both.
   - **`PTRACE_GETREGS`/`SETREGS`** — superseded by `GETREGSET`/`SETREGSET`
     (see above); no reason to carry both.

## Alternatives considered

- **Decline outright, same as `flock`/`chroot`/`sendfile`.** This is the
  default Round 4 would apply on "no confirmed consumer need" grounds alone,
  and remains available: nothing above is implemented yet, and this ADR's
  Decision only takes effect once accepted. Recorded as the honest baseline
  this proposal is measured against, not silently skipped the way ADR-0003
  recorded its own "defer" alternative.
- **A single `ptrace(request, pid, addr, data)` 1:1 passthrough**, matching
  raw libc's shape exactly. Rejected: unlike every other syscall this crate
  wraps, `ptrace`'s argument meaning is entirely request-dependent, so a
  thin passthrough gives a safe caller nothing to hold onto — no way to
  express "this request needs an address, that one needs a data word, this
  other one needs neither" in the type system. Every other module in this
  crate exposes a typed function per operation; `ptrace` gains more from
  that pattern than it loses from the extra function count.
- **`PTRACE_GETREGS`/`SETREGS` instead of `GETREGSET`/`SETREGSET`.**
  Rejected: `GETREGS` is arch-specific in both the struct layout *and* the
  presence/absence of the request itself on some architectures, whereas
  `GETREGSET` is the modern, portable interface the kernel documents as the
  one new code should use — confining the arch divergence to just `GpRegs`
  is strictly better than letting it leak into the call shape as well.
- **Defer `PTRACE_SYSCALL`-based syscall tracing to a future issue.** This
  was this ADR's own original recommendation (see Context) — overridden.
  Recorded here rather than silently dropped, the same way ADR-0003
  recorded its own overridden "defer" position: the extra work this pulls
  in (the `wait4`-driven entry/exit-stop loop, the `orig_rax`/`x8`
  syscall-number reasoning, `PTRACE_O_TRACESYSGOOD`'s required companion
  call) is real, not hand-waved away, but the decision is to take it on now
  rather than leave `ptrace` without the one capability (`strace`-style
  tracing) most concretely motivates having `ptrace` at all.
- **A dedicated crate-side state machine tracking entry-vs-exit stop
  parity**, instead of leaving it to the caller. Rejected: every other
  stateful protocol in this crate (the `wait`/`waitpid` reap cycle itself,
  for one) already leaves sequencing to the caller rather than hiding it
  behind an implicit tracked state the crate owns — a tracer already has to
  drive its own `wait4` loop regardless, so tracking one `bool` alongside
  that loop is a small addition to state the caller manages either way, not
  a new burden.
- **Fold `ptrace` into `process.rs`.** Rejected on the same file-size/
  single-concern grounds that already split `mmap`, `socket`, `epoll`,
  `eventfd`, `inotify`, and `sysinfo` into their own modules rather than
  growing `process.rs` further.

## Consequences

- New module: `ptrace`, tracked as its own implementation issue once this
  ADR is accepted (mirroring how ADR-0003 tracked `socket`/`dns` as separate
  implementation issues rather than one PR).
- New per-arch syscall number: `PTRACE` (a single syscall number shared by
  every request in both the in-scope and out-of-scope lists above — the
  request constant, not the syscall number, is what varies).
- Two new per-arch structs (`GpRegs` for x86_64 and aarch64), each with its
  own kernel-source-verified size/offset const assertions, the same rigor
  `EpollEvent` and `SockAddrIn`/`SockAddrIn6` already meet.
- This is a larger surface than any single-issue addition shipped so far in
  the parity-loop sweep (tracer lifecycle, portable register access, *and*
  syscall tracing in one ADR) — likely worth splitting across more than one
  implementation PR (e.g. lifecycle + register access first, syscall
  tracing as a follow-on building on both), the same incremental-shipping
  pattern ADR-0003's `socket`/`dns` split already used, rather than one
  single PR carrying the whole module.
- Thread affinity: the kernel requires the *specific thread* that attached
  (or received `TRACEME`) to be the one issuing further `ptrace` calls for
  that tracee, not just any thread in the tracer process — an OS-level
  detail unlike most syscalls this crate wraps. The module doc and each
  function's `# Safety`/usage note need to say so plainly, the same way
  `process::fork`'s doc already calls out its own sharp edges.
