# rusty_win32

A `#![no_std]`-where-possible, minimal-dependency, **Windows-only** Rust
crate that gives [rush](https://github.com/baileyrd/rush) a `sys::win32`
backend — the Windows counterpart to
[rusty_libc](https://github.com/baileyrd/rusty_libc), which does the same
job for Linux. Same philosophy, different platform: Windows guarantees no
stable syscall numbers, only stable, documented DLL exports, so this crate
is `extern "system"` FFI against `kernel32.dll` (and later `advapi32.dll`),
not raw `asm!` — see the crate's own module docs (`src/lib.rs`) for why this
isn't a port of `rusty_libc`'s architecture.

## Status

Past its original five-phase plan — every phase below shipped, plus two
full follow-on batches (a needs-driven "round 2" of previously out-of-scope
subsystems, then a systematic parity-loop sweep against the real Win32 API
surface). `src/lib.rs`'s own module docs are the authoritative,
continuously updated design log (why each module exists, in the order it
was added); this is a snapshot of the current module set, not a substitute
for that log.

- [`error::Win32Error`] — a `GetLastError()` wrapper with named `ERROR_*`
  constants, `Display`, `core::error::Error`, `FormatMessageW`-backed full
  message text, and an opt-in `std` feature adding
  `From<Win32Error> for std::io::Error`.
- [`console`] — Ctrl-handler installation (`SetConsoleCtrlHandler`/
  `GenerateConsoleCtrlEvent`), raw-mode primitives (`GetConsoleMode`/
  `SetConsoleMode`/`ReadFile`/`wait_readable`/`window_size`), console
  codepage, and synthetic character/key-event input for tests.
- [`handle`] — `DuplicateHandle`/`CreatePipe`/`SetHandleInformation`/
  `PeekNamedPipe`/`CloseHandle`.
- [`dynlib`] — `LoadLibraryW`/`GetProcAddress`/`FreeLibrary`, for a DLL not
  linked at build time.
- [`process`] and [`job`] — `spawn_suspended`/`resume`/`wait`/`wait_any`,
  process groups + scoped Ctrl-Break, `TerminateProcess`/`OpenProcess`-by-pid,
  `GetProcessTimes`, environment-block overrides and snapshotting, process/
  thread enumeration, and the full Windows Job Object lifecycle including
  completion-port notifications and CPU/IO resource limits.
- [`time`] — `now_monotonic`/`now_realtime` via
  `QueryPerformanceCounter`/`GetSystemTimePreciseAsFileTime`.
- [`path`] — `PATHEXT`-aware command resolution, current-directory
  get/set, and short/long path conversion.
- [`fs`] — `stat`, symlink creation, and related file primitives.
- [`pipe`] — named pipes (`CreateNamedPipeW`/`ConnectNamedPipeW`/
  `WaitNamedPipeW`).
- [`volume`] — drive/volume enumeration.
- [`watch`] — `ReadDirectoryChangesW`, over `OVERLAPPED` I/O.
- `registry`, `security`, `service`, `net` — round-2 subsystems: registry
  value CRUD, file/directory ACLs, service control, and TCP/UDP sockets.
- `conpty` — the pseudoconsole lifecycle (`CreatePseudoConsole`/
  `ResizePseudoConsole`/`ClosePseudoConsole`) plus
  `process::spawn_suspended_with_pseudoconsole`, for hosting a
  fully-interactive child (a terminal-emulator use case, distinct from
  `rusty_lines`' own-process raw-mode reads above).
- `windowing` — User32/GDI32 window, message, and framebuffer bindings.
- `pe` — a zero-`unsafe` parser for the on-disk layout of a Portable
  Executable image (headers, sections, data directories, exports,
  imports) — the read half of a PE loader. Takes a `&[u8]` rather than a
  live handle, so unlike every other module here it's not
  `#[cfg(windows)]`-gated; it stops short of mapping/relocating/running
  an image in-process, which stays the OS loader's job via
  `process`/`dynlib`.

`docs/archive/gap-analysis.md` and `docs/archive/CAPABILITY_ASSESSMENT.md`
are both closed-out gap sweeps, archived once every item either shipped or
was consciously deferred with a reason recorded inline — kept for the
historical record, not as a live backlog. See
`docs/WINDOWS_BACKEND_ANALYSIS.md` in the rush repo for that project's own
side of the primitive-by-primitive analysis this crate is built against.

## Testing

Real behavioral testing needs a Windows machine — this crate is developed
from a Linux sandbox with no way to execute a Windows binary, so:

- `cargo test`/`cargo clippy` on the host target cover the platform-neutral
  logic that doesn't touch a real Win32 call (`error.rs`'s `Display`/
  `From<Win32Error> for std::io::Error` logic).
- `cargo check --target x86_64-pc-windows-gnu` (via `mingw-w64`, see
  `.cargo/config.toml`) is a fast compile-only sanity check for everything
  else, including `console.rs`'s `#[cfg(windows)]`-gated code.
- The real gate is CI's `windows-latest` job, which actually runs every
  test, `console.rs`'s included.

## License

MIT
