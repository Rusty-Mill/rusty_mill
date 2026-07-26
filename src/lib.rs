//! `rusty_win32` — a `#![no_std]`-where-possible, minimal-dependency Windows
//! backend for [rush](https://github.com/baileyrd/rush)'s `sys::win32`, the
//! Windows counterpart to
//! [rusty_libc](https://github.com/baileyrd/rusty_libc) (Linux). Not a
//! shared codebase with `rusty_libc` — Windows has no equivalent of "a
//! stable syscall number," so this crate's primitive is "a documented DLL
//! export" reached via `extern "system"` FFI, not raw `asm!` syscalls. See
//! `docs/WINDOWS_BACKEND_ANALYSIS.md` in the rush repo for the
//! primitive-by-primitive analysis this crate's module boundaries and
//! phasing are derived from.
//!
//! Phase 1: [`error::Win32Error`] and [`console::install_ctrl_handler`] —
//! closing rush's single highest-value, lowest-risk Windows gap identified
//! by that analysis: `trap 'cmd' TERM` is silently accepted on Windows today
//! but has nothing installed to ever fire it.
//!
//! Phase 2: [`handle`] —
//! `DuplicateHandle`/`CreatePipe`/`SetHandleInformation`/`CloseHandle`, the
//! primitive rush's own fd-3-and-up gap needs (rush still has to grow its
//! own integer-to-`HANDLE` map on top; this crate provides the raw
//! primitives, not that map).
//!
//! Phase 3: [`process`] and [`job`] — `spawn_suspended`/`resume`/`wait` (a
//! raw `CreateProcessW` path `std::process::Command` can't provide, needed
//! only for the suspend-then-assign-to-job sequencing below) and Windows
//! Job Objects
//! (`CreateJobObjectW`/`AssignProcessToJobObject`/`SetInformationJobObject`/
//! `TerminateJobObject`/`QueryInformationJobObject`), the primitives rush's
//! own `docs/WINDOWS_JOB_CONTROL.md` designs its background-job tracking
//! (`&`, `jobs`, `wait`, `kill`, `$!`) against.
//!
//! Phase 5: [`time`] — `now_monotonic`/`now_realtime` via
//! `QueryPerformanceCounter`/`GetSystemTimePreciseAsFileTime`, a genuine
//! parallel to `rusty_libc::vdso`'s "read kernel-shared memory instead of
//! syscalling" trick. Lowest priority in the analysis doc: no rush call
//! site needs it (rush uses `std::time` exclusively, and std's own Windows
//! backend already uses `QueryPerformanceCounter` internally) — this
//! exists for `rusty_lines`/completeness, not an open rush gap.
//!
//! Phase 4 (this crate's current state — implemented *after* Phase 5,
//! once its actual shape was confirmed rather than guessed): raw-mode
//! primitives added to [`console`] —
//! [`console::get_mode`]/[`console::set_mode`] (`GetConsoleMode`/
//! `SetConsoleMode`), [`console::read`] (`ReadFile`), [`console::wait_readable`]
//! (`WaitForSingleObject`), and [`console::window_size`]
//! (`GetConsoleScreenBufferInfo`). **Deliberately not ConPTY**: the
//! original handoff doc's sketch ("`CreatePseudoConsole`-backed raw mode")
//! turned out to be the wrong primitive after actually reading
//! `rusty_lines`' source rather than assuming — `CreatePseudoConsole` hosts
//! a *child* process's console session (what a terminal emulator does);
//! `rusty_lines` reads from its own inherited stdin, exactly the way the
//! Unix backend calls `tcgetattr`/`tcsetattr` on its own fd. The real
//! analog of `tcgetattr`/`tcsetattr` is `GetConsoleMode`/`SetConsoleMode`,
//! and `ENABLE_VIRTUAL_TERMINAL_INPUT` (Windows 10+, within this crate's
//! existing floor) is what makes `ReadFile` on a console handle deliver a
//! Unix-tty-like VT/ANSI byte stream instead of requiring
//! `ReadConsoleInputW`'s structured records. This also corrected a factual
//! error the original phasing carried: rush's own
//! `docs/WINDOWS_BACKEND_ANALYSIS.md` had claimed Windows Ctrl-C-at-idle-prompt
//! already worked via `rusty_lines`; it doesn't — `rusty_lines`' non-Unix
//! path has no Ctrl-C handling at all today (see that doc's own
//! corrected text). As with `rusty_libc`'s `tcgetattr`/`tcsetattr`, this
//! crate exposes the primitive only — deciding which mode bits constitute
//! "raw mode" is `rusty_lines`' policy, not this crate's.
//!
//! [`console::write_char_events`] (`WriteConsoleInputW`) followed later,
//! for a different reason than the phases above: not a rush or
//! `rusty_lines` production need, but the primitive a *test* needs to
//! synthesize real console input and drive a raw-mode reader through its
//! real Windows I/O path end to end — the Windows analog of writing bytes
//! into one end of a Unix pty, without needing ConPTY (which this crate
//! still doesn't have — see the "Deliberately not ConPTY" note above for
//! why that's the right call for `rusty_lines`' own reads, and note this
//! primitive is a narrower thing: synthesizing input into an *existing*
//! console this process already owns, not hosting a *child* process's
//! console session). Its own test empirically proves the
//! `WriteConsoleInputW` → `ENABLE_VIRTUAL_TERMINAL_INPUT` → `ReadFile`
//! round trip actually produces the same bytes a real keypress would,
//! rather than assuming it.
//!
//! [`process::environment_block`] followed for rush's own first real
//! consumer of Phase 3: `rush`'s `vars` module never calls
//! `std::env::set_var`/`remove_var` (it keeps its own exported-variable
//! table instead — see that crate's `expand.rs`), so after any
//! `export`/`unset` the real OS environment `spawn_suspended` used to
//! inherit by default would silently diverge from what `rush` itself
//! believes a child should see. `spawn_suspended` now takes an optional
//! environment block built by this function, so a caller tracking its own
//! variable table can hand `CreateProcessW` a from-scratch one instead of
//! relying on inheritance — needed for `rush`'s upcoming Windows
//! background-job support (`docs/WINDOWS_JOB_CONTROL.md` in the rush repo),
//! where the spawned child is a `CREATE_SUSPENDED` process this crate
//! builds directly, not a `std::process::Command` that could otherwise
//! just call `.env_clear()`/`.envs()` itself.
//!
//! [`job::clear_kill_on_close`] followed for the same consumer's `disown`
//! builtin: a job created via [`job::create`]/[`job::set_kill_on_close`]
//! ties its member processes' lifetime to the job handle, which closes
//! implicitly at the owning process's own exit exactly like any other
//! handle — so a caller can't just stop tracking a job and drop its
//! handle to detach a process from the shell's lifetime; the kill-on-close
//! limit itself has to be reversed first, or the process dies anyway.
//!
//! [`process::wait_any`] followed for the same consumer's `wait -n`: without
//! it, blocking on "whichever of several tracked background jobs finishes
//! first" had no primitive to build on beyond looping [`process::wait`]
//! with a zero timeout over every handle in turn and sleeping between
//! sweeps — a real but coarser stand-in `docs/WINDOWS_JOB_CONTROL.md`
//! explicitly flagged as a follow-up once this existed.
//! `WaitForMultipleObjects` (`bWaitAll = FALSE`) is the actual OS primitive
//! for that; this wrapper's contract mirrors `process::wait`'s (same
//! `Option<u32>` timeout convention, same exit-code fetch via
//! `GetExitCodeProcess`), just over a slice of handles instead of one.
//!
//! [`path::resolve_command`] (backed by [`path::search_path`], a
//! `SearchPathW` wrapper) closes a gap the capability assessment
//! (`docs/CAPABILITY_ASSESSMENT.md`) flagged as this crate's single biggest
//! remaining *correctness* gap, not merely a nice-to-have: Windows has no
//! executable bit the way Unix does, so "is `foo` runnable" is answered
//! entirely by file extension plus the `PATHEXT` environment variable
//! (`.COM;.EXE;.BAT;.CMD;...`) instead of a `stat` mode check. Without this,
//! a bare command name (`foo`, as opposed to `foo.exe`) has no primitive to
//! resolve against on Windows at all. `pathext` is caller-supplied, the same
//! way `spawn_suspended`'s `environment` parameter is, rather than read from
//! the real environment out from under a caller tracking its own variable
//! table.
//!
//! [`job::associate_completion_port`]/[`job::wait_for_message`] close
//! another gap the same assessment flagged: [`job::process_ids`] is a poll,
//! never a push — there was no Unix-`SIGCHLD` equivalent for "a job member
//! just exited" without looping that poll on a timer. Windows repurposes
//! I/O completion ports (otherwise a file-I/O mechanism, not a process-
//! lifecycle one) for job notifications instead of defining a job-specific
//! primitive, so that's what this crate wraps, rather than inventing its
//! own polling loop as a permanent stand-in.
//!
//! [`fs::stat`]/[`fs::stat_by_handle`] (`GetFileAttributesExW`/
//! `GetFileInformationByHandle`) close another capability-assessment gap:
//! Windows has no `stat`/`fstat` equivalent in this crate at all yet, and
//! `ls`, globbing, and the `[ -d/-f ]` test operators all need one
//! eventually. `stat_by_handle` additionally reports a volume serial number
//! and file index — Windows' closest analog of a Unix `(st_dev, st_ino)`
//! pair — which a bare path can't answer without opening a handle first.
//!
//! [`fs::create_symlink`]/[`fs::readlink`]/[`fs::final_path`]
//! (`CreateSymbolicLinkW`/`DeviceIoControl(FSCTL_GET_REPARSE_POINT)`/
//! `GetFinalPathNameByHandleW`) round out `fs` with `ln -s` parity and
//! reparse-point resolution. `readlink` only parses the symlink variant of
//! `REPARSE_DATA_BUFFER` — an NT-native structure with no ordinary Win32
//! API, verified against mingw-w64's DDK headers rather than the usual
//! Win32 set — junctions/mount points are a deliberate scope cut, not an
//! oversight.
//!
//! [`process::environment_snapshot`] (`GetEnvironmentStringsW`) closes the
//! read-back gap [`process::environment_block`] left open: that function
//! only ever *builds* a block for a spawned child, with no way for a
//! caller to *seed* its own variable table from the real environment this
//! process actually inherited at startup — needed once, by `rush`'s `vars`
//! module, before it starts tracking exports/unsets on its own.
//!
//! [`error::Win32Error::message`] (`FormatMessageW`) covers the gap
//! `Display`'s fixed, hand-maintained table always had: anything outside
//! its ~25 named codes prints "unknown Win32 error N" even though Windows
//! itself can usually describe the code. `message` is the slower,
//! allocating fallback for that case — `Display`'s table stays the fast,
//! allocation-free path for the common/named codes, rather than replacing
//! it outright.
//!
//! [`process::list_processes`] (`CreateToolhelp32Snapshot`/
//! `Process32FirstW`/`Process32NextW`) rounds out the capability
//! assessment's remaining lower-priority items with a `ps`-equivalent:
//! a point-in-time system-wide process snapshot, not previously possible
//! with anything in this crate.
//!
//! [`process::spawn_suspended`]'s `new_process_group` parameter and
//! [`console::generate_ctrl_event`] close the round-2 capability
//! assessment's top-ranked gap: nothing here previously let a caller
//! interrupt *one* child without affecting every process attached to the
//! console at once. `CTRL_C_EVENT` can only ever be broadcast console-wide
//! by Windows' own design — `CREATE_NEW_PROCESS_GROUP` plus
//! `CTRL_BREAK_EVENT` is the actual mechanism a console app uses to target
//! one child (and its descendants), so that's what's exposed here rather
//! than a `CTRL_C_EVENT`-shaped API that couldn't do what its name implies.
//!
//! [`process::times`] (`GetProcessTimes`) closes the round-2 assessment's
//! other must-have: without it, rush's `time` builtin has no way to report
//! real per-child CPU time on Windows at all and falls back to a hardcoded
//! zero (a visibly wrong `time` output, not merely a missing one, on every
//! non-Linux target). `kernel_time`/`user_time` reuse [`time::Timespec`]'s
//! shape for an elapsed *duration* rather than a wall-clock timestamp —
//! the same reuse [`time::now_monotonic`]'s result already relies on.
//!
//! [`job::set_resource_limits`]/[`job::limits`]/[`job::accounting`] close
//! the round-2 assessment's Job-Object item: rush's `ulimit` is flat "not
//! supported" on Windows today, and Job-Object memory/CPU-time/
//! active-process limits are that doc's own answer for the only realistic
//! partial fix — the struct fields these use were already modeled
//! bit-for-bit for [`job::set_kill_on_close`], just never set beyond its
//! one `LimitFlags` bit until now.
//!
//! [`pipe`] (`CreateNamedPipeW`/`ConnectNamedPipeW`/`WaitNamedPipeW`/
//! `CreateFileW`) closes the round-2 assessment's remaining Job-Object-
//! adjacent gap by a different route: the one concrete missing primitive
//! blocking rush's already-deferred process substitution (`<(cmd)`) and
//! `coproc` support on Windows, since [`handle::create_pipe`]'s anonymous
//! pipes have no name an arbitrary already-running program can open.
//!
//! [`console::write_key_events`] closes the round-2 assessment's last
//! must-/nice-to-have item, extending [`console::write_char_events`]'s
//! test-input-synthesis technique to the one case its own doc names as out
//! of scope: non-character keys (arrows, Home/End, function keys, …) that
//! carry no `uChar` at all. This crate's first non-`kernel32` link
//! (`user32.dll`'s `MapVirtualKeyW`, for a real hardware scan code) — the
//! README's own module docs already flagged `advapi32.dll` as an expected
//! future addition; this is the same kind of expansion.
//!
//! [`volume`] (`GetLogicalDrives`/`GetDriveTypeW`/`GetVolumeInformationW`)
//! rounds out the round-2 assessment's remaining speculative items: a
//! distinctly Windows-shaped gap (Windows' multi-root filesystem model has
//! no Unix analog at all) flagged for completeness rather than because any
//! current `rush`/`rusty_lines` call site needs it yet.
//!
//! [`path::short_path`]/[`path::long_path`] (`GetShortPathNameW`/
//! `GetLongPathNameW`) close one of the round-2 assessment's last
//! speculative items: normalizing between a legacy 8.3 short name and its
//! long form — a rare but real source of path-comparison surprises this
//! crate's reparse-point-aware [`fs::final_path`] doesn't otherwise cover,
//! again with no known consumer today.
//!
//! [`watch`] (`ReadDirectoryChangesW`) closes the round-2 assessment's
//! final item, and the only one that genuinely required `OVERLAPPED` I/O
//! — every other primitive in this crate got away with a purely
//! synchronous call, but `ReadDirectoryChangesW` has no way to bound how
//! long it blocks otherwise. [`watch::read_changes`] wraps the overlapped
//! path behind the same `Option<u32>` timeout convention
//! [`process::wait`] already uses, cancelling the pending read via
//! `CancelIoEx` on timeout rather than leaving a caller with no way to
//! ever give up waiting.
//!
//! [`path::current_dir`]/[`path::set_current_dir`] (`GetCurrentDirectoryW`/
//! `SetCurrentDirectoryW`) come from a parity-loop pass against the real
//! Win32 API surface (`gap-analysis.md`), not the round-2 capability
//! assessment above — a systematic function-level sweep rather than a
//! needs-driven inventory. This particular gap turned out to be the most
//! surprising finding in that sweep: the actual Win32 primitives behind
//! `cd`/`pwd`, and nothing in this crate wrapped them at all until now.
//!
//! Safe wrappers return `Result<T, Win32Error>`; a raw Win32 error code
//! never escapes unwrapped. `unsafe` is confined to the `extern "system"`
//! FFI declarations and functions that take a caller-supplied raw handle or
//! an unquoted command line (`handle`'s and `job`'s handle-taking
//! functions, `console::write_char_events`,
//! `process::spawn_suspended`/`resume`/`wait`/`wait_any`) — everything else
//! is safe.

#![cfg_attr(not(any(test, feature = "std")), no_std)]

pub mod error;
pub use error::Win32Error;

#[cfg(windows)]
pub mod console;
#[cfg(windows)]
pub use console::{HandlerRoutine, generate_ctrl_event, install_ctrl_handler, remove_ctrl_handler};

#[cfg(windows)]
pub mod handle;
#[cfg(windows)]
pub use handle::{RawHandle, close, create_pipe, duplicate, pipe_bytes_available, set_inheritable};

#[cfg(windows)]
pub mod process;
#[cfg(windows)]
pub use process::{
    MAXIMUM_WAIT_OBJECTS, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE, ProcessEntry,
    ProcessTimes, SYNCHRONIZE, SpawnedProcess, current_pid, environment_block,
    environment_snapshot, list_processes, open_by_pid, resume, spawn_suspended, terminate, times,
    wait, wait_any,
};

// `job`'s six-item surface (`create`/`assign`/`set_kill_on_close`/
// `clear_kill_on_close`/`terminate`/`process_ids`) is deliberately *not*
// re-exported at the crate root, unlike the smaller modules above: a job's
// lifecycle is always used as a cohesive group of calls against the same
// handle, not as a single flagship function reached in isolation, so
// flattening all six into the root namespace would add noise without
// adding ergonomics. Reach it via `rusty_win32::job::*`.
#[cfg(windows)]
pub mod job;

#[cfg(windows)]
pub mod time;
#[cfg(windows)]
pub use time::{Timespec, now_monotonic, now_realtime};

#[cfg(windows)]
pub mod path;
#[cfg(windows)]
pub use path::{current_dir, long_path, resolve_command, search_path, set_current_dir, short_path};

// `fs`'s several-item surface (two functions, two result structs, and the
// `FILE_ATTRIBUTE_*` constants) is deliberately not re-exported at the
// crate root either, for the same reason as `job`'s — reach it via
// `rusty_win32::fs::*`.
#[cfg(windows)]
pub mod fs;

// `pipe`'s several-item surface (four functions plus the `PIPE_*`/
// `GENERIC_*` mode constants) is deliberately not re-exported at the crate
// root either, for the same reason as `job`'s/`fs`'s — reach it via
// `rusty_win32::pipe::*`.
#[cfg(windows)]
pub mod pipe;

// `volume`'s several-item surface (three functions, a result struct, and
// the `DriveType` enum) is deliberately not re-exported at the crate root
// either, for the same reason as `job`'s/`fs`'s/`pipe`'s — reach it via
// `rusty_win32::volume::*`.
#[cfg(windows)]
pub mod volume;

// `watch`'s several-item surface (two functions, a result struct, and the
// `FILE_NOTIFY_CHANGE_*`/`FILE_ACTION_*` constants) is deliberately not
// re-exported at the crate root either, for the same reason as
// `job`'s/`fs`'s/`pipe`'s/`volume`'s — reach it via `rusty_win32::watch::*`.
#[cfg(windows)]
pub mod watch;

// `registry` is a brand-new round-2 subsystem (previously excluded by this
// crate's own non-goals — see `gap-analysis.md`'s "Round 2: previously
// out-of-scope subsystems" sweep), starting from just the `HKey` type and
// the five predefined root keys. Not re-exported at the crate root, for
// the same reason as `job`'s/`fs`'s/`pipe`'s/`volume`'s/`watch`'s — reach
// it via `rusty_win32::registry::*`.
#[cfg(windows)]
pub mod registry;

// `security` is another brand-new round-2 subsystem (same previously-
// excluded-by-non-goals history as `registry`), starting from the core
// path → owner/DACL round trip. Not re-exported at the crate root, for
// the same reason as `registry`'s and the others above — reach it via
// `rusty_win32::security::*`.
#[cfg(windows)]
pub mod security;

// `service` is a third brand-new round-2 subsystem (same previously-
// excluded-by-non-goals history as `registry`/`security`) — a
// `systemctl`-equivalent: list/query/start/stop a named Windows service.
// Starts with just the SCM/service handle lifecycle. Not re-exported at
// the crate root, for the same reason as the others above — reach it via
// `rusty_win32::service::*`.
#[cfg(windows)]
pub mod service;

// `net` is a fourth brand-new round-2 subsystem (same previously-
// excluded-by-non-goals history as `registry`/`security`/`service`) —
// basic TCP/UDP client+server socket programming, the same core subset
// `rusty_libc` wraps for POSIX sockets. Starts with Winsock's own
// load/unload lifecycle. Not re-exported at the crate root, for the same
// reason as the others above — reach it via `rusty_win32::net::*`.
#[cfg(windows)]
pub mod net;

// `conpty` is a fifth brand-new round-2 subsystem, excluded until now for
// an architectural-boundary reason rather than a "no consumer" one:
// `rusty_lines` reads its own inherited stdin rather than hosting a
// child's terminal. Starts with the pseudoconsole lifecycle itself. Not
// re-exported at the crate root, for the same reason as the others above
// — reach it via `rusty_win32::conpty::*`.
#[cfg(windows)]
pub mod conpty;
#[cfg(windows)]
pub mod windowing;
