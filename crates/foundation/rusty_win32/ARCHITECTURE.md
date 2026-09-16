# Architecture

## Overview
`rusty_win32` is a `#![no_std]`-where-possible, minimal-dependency library
crate providing [rush](https://github.com/baileyrd/rush)'s `sys::win32`
backend — the Windows counterpart to
[rusty_libc](https://github.com/baileyrd/rusty_libc) (Linux). It has no
runtime of its own: it's a leaf dependency, linked into `rush` (and
secondarily [rusty_lines](https://github.com/baileyrd/rusty_lines)), never run
standalone.

**Non-goals:** a `std::process::Command`/`std::fs` replacement (use those
directly wherever they already work — this crate exists only for the gap
they can't cover, e.g. `CREATE_SUSPENDED` spawning); a portable abstraction
layer (no `cfg(unix)` branch anywhere in this crate — that split lives one
level up, in `rush`'s own `sys` module). Most modules exist because a
specific `rush`/`rusty_lines` call site needs them; a smaller "round 2" set
(see `docs/archive/gap-analysis.md`, closed out and archived — every item
shipped) was added for Win32 parity/completeness on explicit direction,
without a current consumer — those are called out as such in `lib.rs`'s
module docs rather than pretending a call site exists.

## Boundaries
Not a ports-and-adapters web service — there's no domain logic to keep free
of I/O, since I/O *is* the domain here. The boundary that matters instead is
"safe wrapper" (the port `rush`/`rusty_lines` actually call) vs. "raw Win32
FFI" (the adapter, an `unsafe extern "system"` declaration against a
`kernel32.dll` export):

| Port (safe wrapper module) | Adapter (Win32 API) | Notes |
| --- | --- | --- |
| `error::Win32Error` | `GetLastError` | Every other module's failure path funnels through this; a raw code never escapes unwrapped. |
| `console` (ctrl handler) | `SetConsoleCtrlHandler` | Process-wide handler chain; closest analog to `SIGINT`/`SIGTERM`/`SIGHUP` delivery. |
| `console` (raw mode) | `GetConsoleMode`/`SetConsoleMode`/`ReadFile`/`WaitForSingleObject`/`GetConsoleScreenBufferInfo`/`WriteConsoleInputW` | Analog of `tcgetattr`/`tcsetattr`/raw `read`/`poll`/`TIOCGWINSZ`; policy-free (what "raw mode" means is the `rusty_lines` caller's decision). |
| `handle` | `CreatePipe`/`DuplicateHandle`/`SetHandleInformation`/`CloseHandle` | Analog of `pipe2`/`dup`/`close`; the integer-to-`HANDLE` map for fd 3+ stays in `rush`, not here. |
| `process` | `CreateProcessW`/`ResumeThread`/`WaitForSingleObject`/`WaitForMultipleObjects`/`GetExitCodeProcess`/`GetCurrentProcessId` | `CREATE_SUSPENDED` spawn + wait, scoped to what Job-Object-integrated background jobs need — not a general process-spawning API. |
| `job` | `CreateJobObjectW`/`AssignProcessToJobObject`/`SetInformationJobObject`/`TerminateJobObject`/`QueryInformationJobObject` | Windows Job Objects as the closest analog of a POSIX process group for lifetime management. |
| `time` | `QueryPerformanceCounter`/`QueryPerformanceFrequency`/`GetSystemTimePreciseAsFileTime` | Analog of `CLOCK_MONOTONIC`/`CLOCK_REALTIME`; lowest-priority module, kept for `rusty_lines`/completeness. |

## Structure
A single flat library crate, one module per Win32 subsystem (`error`,
`console`, `handle`, `process`, `job`, `time`) — not a modular monolith in
the service sense, since there's no deployable unit or service boundary to
speak of. Each module is independently `#[cfg(windows)]`-gated except
`error`, which stays available off-Windows so downstream crates can still
name `Win32Error` in non-`cfg(windows)` code paths. No module has crossed (or
is expected to cross) into its own crate — the whole point of this crate is
to be one small, auditable FFI surface `rush` links against.

## Data flow
`rush`/`rusty_lines` call a safe wrapper function → the wrapper builds
correctly-sized/aligned structs and validates its own preconditions where it
can → an `unsafe` block calls the `extern "system"` FFI declaration into
`kernel32.dll` → the raw `BOOL`/handle/code return is checked → success maps
to a typed return value, failure maps to `Win32Error::last()` (or, for
functions that report failure via a sentinel return rather than
`GetLastError`, `Win32Error::from_raw`). No wrapper ever returns a raw Win32
error code or a raw `HANDLE`-shaped success value without going through this
translation.

## Key decisions
See [docs/adr/](./docs/adr/) for the record of individual decisions and their
tradeoffs. The crate's own module docs (`src/lib.rs`) additionally carry a
phase-by-phase narrative of *why* each module was added, in more depth than
an ADR would normally hold — treat `lib.rs` as the primary running design log
for this crate, with `docs/adr/` reserved for a small number of
higher-level, cross-module decisions.

## Non-goals
- **Command-line quoting/escaping.** `process::spawn_suspended` takes an
  already-built command line; Windows argv quoting is the caller's
  responsibility (`std::process::Command` already solves it correctly, and
  there's no public API to reuse its logic here).
- **Non-Windows targets.** This crate is Windows-only by design; there is no
  portable fallback path anywhere in it.
