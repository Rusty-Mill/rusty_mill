//! Sovereign Environment variables and arguments for rusty_std.

use crate::path::PathBuf;
use alloc::string::String;
use alloc::vec::Vec;

/// Returns the command line arguments passed to the binary.
///
/// Backed by a real OS primitive: `/proc/self/cmdline` (via
/// `rusty_libc::fd`) on Linux, `GetCommandLineW`/`CommandLineToArgvW`
/// (`kernel32`/`shell32`) on Windows -- wired directly to those two raw
/// Win32 APIs since, unlike `net.rs`/`time.rs`, `rusty_win32` does not
/// expose an existing primitive for the current process's own argv. On
/// any other target (e.g. `wasm32`) there is no wired primitive yet, and
/// this returns an empty `Vec` -- callers on those targets must not rely
/// on `args()` for real argv.
pub fn args() -> Vec<String> {
    #[cfg(target_os = "linux")]
    {
        let Ok(fd) = rusty_libc::fd::open(c"/proc/self/cmdline", rusty_libc::fd::O_RDONLY, 0)
        else {
            return Vec::new();
        };
        let mut raw = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            match rusty_libc::fd::read(fd, &mut chunk) {
                Ok(0) => break,
                Ok(n) => raw.extend_from_slice(&chunk[..n]),
                Err(_) => break,
            }
        }
        let _ = rusty_libc::fd::close(fd);
        // `/proc/self/cmdline` is a NUL-separated argv, itself terminated by
        // one trailing NUL -- strip only that terminator so a genuinely
        // empty argument in the middle of argv is preserved rather than
        // dropped by a blanket "skip empty" filter.
        let body = raw.strip_suffix(&[0u8]).unwrap_or(&raw[..]);
        if body.is_empty() {
            Vec::new()
        } else {
            body.split(|&b| b == 0)
                .map(|s| String::from_utf8_lossy(s).into_owned())
                .collect()
        }
    }
    #[cfg(windows)]
    {
        windows_args()
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        Vec::new()
    }
}

/// Reads the current process's own argv via `GetCommandLineW` +
/// `CommandLineToArgvW` -- the standard Win32 pair for recovering argv
/// from a running process, since Windows has no `argc`/`argv` the OS
/// itself hands the runtime the way Unix `execve` does (only one flat
/// command-line string).
#[cfg(windows)]
fn windows_args() -> Vec<String> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCommandLineW() -> *const u16;
        fn LocalFree(mem: *mut core::ffi::c_void) -> *mut core::ffi::c_void;
    }
    #[link(name = "shell32")]
    unsafe extern "system" {
        fn CommandLineToArgvW(cmd_line: *const u16, num_args: *mut i32) -> *mut *mut u16;
    }

    // SAFETY: `GetCommandLineW` returns a pointer to a NUL-terminated
    // UTF-16 string owned by the process, valid for the process's entire
    // lifetime.
    let cmd_line = unsafe { GetCommandLineW() };
    let mut argc: i32 = 0;
    // SAFETY: `cmd_line` is a valid, NUL-terminated wide string from
    // `GetCommandLineW`; `argc` is a valid, exclusively-borrowed out
    // parameter.
    let argv = unsafe { CommandLineToArgvW(cmd_line, &mut argc) };
    if argv.is_null() {
        return Vec::new();
    }
    let mut result = Vec::with_capacity(argc.max(0) as usize);
    for i in 0..argc as isize {
        // SAFETY: `CommandLineToArgvW` returned non-null with `argc`
        // entries; `i` is in `0..argc`.
        let wide_ptr = unsafe { *argv.offset(i) };
        let mut len = 0usize;
        // SAFETY: `wide_ptr` is a valid, NUL-terminated wide string, per
        // `CommandLineToArgvW`'s documented contract.
        while unsafe { *wide_ptr.add(len) } != 0 {
            len += 1;
        }
        // SAFETY: `wide_ptr` points to `len` valid, initialized `u16` code
        // units, established by the NUL-scan above.
        let slice = unsafe { core::slice::from_raw_parts(wide_ptr, len) };
        result.push(String::from_utf16_lossy(slice));
    }
    // SAFETY: `argv` was allocated by `CommandLineToArgvW` and is released
    // with `LocalFree` exactly once here, after every entry has already
    // been copied out into owned `String`s above.
    unsafe {
        LocalFree(argv.cast::<core::ffi::c_void>());
    }
    result
}

/// Returns the temporary directory path for the current system.
pub fn temp_dir() -> PathBuf {
    #[cfg(windows)]
    {
        PathBuf::from("C:\\Temp")
    }
    #[cfg(not(windows))]
    {
        PathBuf::from("/tmp")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_includes_at_least_the_running_binary() {
        let argv = args();
        assert!(
            !argv.is_empty(),
            "args() should report at least argv[0] (the program name) when actually run as a process, not an empty stub"
        );
    }
}
