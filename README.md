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

## Status: Phase 5

- [`error::Win32Error`] — a `GetLastError()` wrapper with named `ERROR_*`
  constants, `Display`, `core::error::Error`, and an opt-in `std` feature
  adding `From<Win32Error> for std::io::Error`.
- [`console::install_ctrl_handler`] / [`console::remove_ctrl_handler`] —
  `SetConsoleCtrlHandler`, closing rush's single highest-value, lowest-risk
  Windows gap: `trap 'cmd' TERM` is accepted but silently never fires today,
  for lack of anything to install a handler.
- [`handle`] — `DuplicateHandle`/`CreatePipe`/`SetHandleInformation`/
  `CloseHandle`, the primitive rush's own fd-3-and-up gap needs.
- [`process`] and [`job`] — `spawn_suspended`/`resume`/`wait`/`wait_any`,
  environment-block overrides for a spawned child, and the full Windows Job
  Object lifecycle (`create`/`assign`/`set_kill_on_close`/
  `clear_kill_on_close`/`terminate`/`process_ids`) rush's background-job
  tracking (`&`, `jobs`, `wait`, `kill`, `$!`) is built against.
- [`console`] raw-mode primitives — `get_mode`/`set_mode`
  (`GetConsoleMode`/`SetConsoleMode`), `read` (`ReadFile`), `wait_readable`
  (`WaitForSingleObject`), `window_size` (`GetConsoleScreenBufferInfo`), and
  `write_char_events` (`WriteConsoleInputW`, for driving a raw-mode reader
  with synthetic input in tests).
- [`time`] — `now_monotonic`/`now_realtime` via
  `QueryPerformanceCounter`/`GetSystemTimePreciseAsFileTime`.

This status list lags the code — the crate has since grown a round-2 batch
of subsystems (`registry`, `security`, `service`, `net`, `conpty`,
`windowing`) plus a `pe` Portable-Executable parser; `src/lib.rs`'s module
docs are the running, up-to-date design log. In particular ConPTY *is* now
implemented (`conpty::create`/`resize`/`close` and
`process::spawn_suspended_with_pseudoconsole`), for hosting a fully-
interactive child rather than `rusty_lines`' own-process raw-mode reads —
the two are different needs, as those module docs explain. The `pe` module
is the read half of a PE loader (headers, sections, exports, imports over a
`&[u8]`); it stops short of mapping/relocating/running an image, which stays
the OS loader's job via `process`/`dynlib`.

See `docs/CAPABILITY_ASSESSMENT.md` in this repo, and
`docs/WINDOWS_BACKEND_ANALYSIS.md` in the rush repo, for the full
primitive-by-primitive analysis and remaining gaps this crate is being built
against.

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
