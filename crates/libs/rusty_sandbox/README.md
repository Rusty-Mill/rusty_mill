# rusty_sandbox

Fail-closed sandboxed execution for untrusted processes, hoisted from
`rusty_rsi` (ADR-0005 §4) in ADR-0007 follow-ons step 7 so that a bot can
be confined the way an untrusted task solution is.

## Shape

| Piece | What it does |
| --- | --- |
| `SandboxSpec`, `Limits` | Where a process may read and write (absolute roots; `cwd` under a write root), its complete environment, and hard limits: CPU, wall clock, address space, file size, open files, processes. Validated on construction; `can_reach` proves a path is outside the sandbox. |
| `Executor` | The port: run one program under a spec and report an `ExecOutcome` (`Exited`, `Signaled` or `TimedOut`, bounded stdout and stderr, wall time). A program that fails is an outcome; a sandbox that cannot be set up is an `Error::Sandbox` and nothing runs. |
| `ProcessExecutor` | The Linux adapter. Spawns a helper binary in its own process group with a clean environment, captures bounded output, kills the whole group at the wall-clock limit, and turns a non-empty status file into a fail-closed error. `exec_with` adds a standard input and a stricter socket rule. `start` returns the running `Job` instead of waiting, for a long-lived process; its `JobHandle` kills the group from any thread and `Job::wait` reaps it. Off Linux every run is refused. |
| `helper` | What the helper binary runs: `run_helper(args)` decodes a `HelperRequest`, sets rlimits, confines the filesystem with Landlock, blocks sockets with seccomp per `Sockets` (`Internet`, `InternetOnly`, `NoInternet`, `NoEndpoints`, `None`), locks the process group and refuses `io_uring`, verifies each step was enforced, then `exec`s. |

`NoInternet` and `Internet` leave local Unix and abstract sockets open (Landlock does not cover them), so a
program that must not reach a local daemon uses `NoEndpoints`, or `InternetOnly` when it needs the network
(IPv4/IPv6 sockets only). Both modes also restrict `socketpair(2)` to stream and seqpacket pairs: a descriptor from a
datagram pair can be re-`connect`ed to any reachable datagram endpoint, which would bypass the socket filter, while a
connection-oriented pair refuses it (`EISCONN`).

Landlock and seccomp apply to the calling thread and its descendants, and
installing them after `fork` in a multithreaded process is unsafe, so the
executor never confines itself: the helper is a single-threaded binary
from birth. Any binary that hands its arguments to `run_helper` will do;
`rsi __sandbox` and `rusty-bot __sandbox` are two.

## Dependencies

First-party only: `rusty_err`, and on Linux `platform`, `platform-linux`
(Landlock and seccomp) and `rusty_libc` (rlimits, signals, the raw
`prctl`).

`start_registered` supplies the cancellation handle immediately after spawn,
including while setup is still pending. The executor retains the job through
all setup checks: a timeout, registration error, helper-resolution error, or
status-file read/removal error kills, contains and reaps before returning.
Dropping an unwaited `Job` also performs cleanup; `wait` retains the same guard
on errors. The production setup deadline remains ten seconds.

Startup regressions use deliberately delayed synthetic helpers through the
actual process executor. They verify containment/reaping and forbid a delayed
launch after timeout or rejection without depending on Landlock availability.
These tests do not establish supported-kernel sandbox enforcement.
