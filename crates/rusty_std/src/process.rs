//! Sovereign Process management for rusty_std.

use crate::error::{Error, Result};
use alloc::string::String;
use alloc::vec::Vec;

#[cfg(target_os = "linux")]
fn map_errno(op: &str, err: rusty_libc::Errno) -> Error {
    Error::Io(err.code(), alloc::format!("{op}: {err}"))
}

#[cfg(windows)]
fn map_win32(op: &str, err: rusty_win32::Win32Error) -> Error {
    Error::Io(err.code() as i32, alloc::format!("{op}: {err}"))
}

/// Command builder for spawning processes.
///
/// Backed by a real OS spawn primitive: `vfork`+`execve`+`waitpid`
/// (`rusty_libc::process`/`rusty_libc::wait`) on Linux,
/// `CreateProcessW`+`WaitForSingleObject`
/// (`rusty_win32::process::spawn_suspended`/`resume`/`wait`) on Windows --
/// `status` genuinely spawns `program` with `args` and waits for its real
/// exit status. On any other target (e.g. `wasm32`) there is no wired
/// spawn primitive yet, and `status` falls back to a stub that always
/// reports success without spawning anything -- callers on those targets
/// must not rely on `status` for real process execution.
pub struct Command {
    program: String,
    args: Vec<String>,
}

impl Command {
    /// Constructs a new Command for launching `program`.
    pub fn new(program: &str) -> Self {
        Self {
            program: String::from(program),
            args: Vec::new(),
        }
    }

    /// Adds an argument to pass to the program.
    pub fn arg(&mut self, arg: &str) -> &mut Self {
        self.args.push(String::from(arg));
        self
    }

    /// Executes the command as a child process, waiting for it to finish.
    pub fn status(&mut self) -> Result<ExitStatus> {
        #[cfg(target_os = "linux")]
        {
            self.status_linux()
        }
        #[cfg(windows)]
        {
            self.status_windows()
        }
        #[cfg(not(any(target_os = "linux", windows)))]
        {
            Ok(ExitStatus { code: 0 })
        }
    }

    /// Spawns and waits for the real child via `vfork`+`execve`+`waitpid`.
    #[cfg(target_os = "linux")]
    fn status_linux(&self) -> Result<ExitStatus> {
        use alloc::ffi::CString;
        use core::ffi::c_char;

        let nul_err = |what: &str| {
            Error::InvalidArgument(alloc::format!("{what} contains an interior NUL byte"))
        };

        let mut owned: Vec<CString> = Vec::with_capacity(self.args.len() + 1);
        owned.push(CString::new(self.program.clone()).map_err(|_| nul_err("program"))?);
        for arg in &self.args {
            owned.push(CString::new(arg.clone()).map_err(|_| nul_err("argument"))?);
        }
        let mut argv: Vec<*const c_char> = owned.iter().map(|s| s.as_ptr()).collect();
        argv.push(core::ptr::null());
        let envp: [*const c_char; 1] = [core::ptr::null()];

        // SAFETY: `argv`/`envp` are valid, null-terminated C-string pointer
        // arrays kept alive by `owned` for the duration of this call.
        let pid =
            unsafe { rusty_libc::process::vfork_exec(&owned[0], argv.as_ptr(), envp.as_ptr()) }
                .map_err(|e| map_errno("vfork_exec", e))?;
        let (_, status) = rusty_libc::wait::waitpid(pid, 0).map_err(|e| map_errno("waitpid", e))?;
        if rusty_libc::wait::wifexited(status) {
            Ok(ExitStatus {
                code: rusty_libc::wait::wexitstatus(status),
            })
        } else {
            // Terminated by a signal (or `vfork_exec`'s own child-side
            // `exit_group(127)` on a failed `execve`, which surfaces here
            // as a normal exit, not this branch): report a signal death
            // the same way a POSIX shell's `$?` does (128 + signal number)
            // rather than a misleading hardcoded 0.
            Ok(ExitStatus {
                code: 128 + rusty_libc::wait::wtermsig(status),
            })
        }
    }

    /// Spawns and waits for the real child via `CreateProcessW` (suspended,
    /// then resumed) + `WaitForSingleObject`.
    #[cfg(windows)]
    fn status_windows(&self) -> Result<ExitStatus> {
        let mut command_line = String::new();
        quote_windows_arg(&self.program, &mut command_line);
        for arg in &self.args {
            command_line.push(' ');
            quote_windows_arg(arg, &mut command_line);
        }

        // SAFETY: `command_line` was just built by `quote_windows_arg`,
        // which quotes/escapes every argument per the standard Windows
        // command-line convention `CommandLineToArgvW` expects.
        let spawned =
            unsafe { rusty_win32::process::spawn_suspended(&command_line, false, false, None) }
                .map_err(|e| map_win32("CreateProcessW", e))?;
        // SAFETY: `spawned.thread` was just created above by
        // `spawn_suspended` and has not yet been resumed or closed.
        unsafe { rusty_win32::process::resume(spawned.thread) }
            .map_err(|e| map_win32("ResumeThread", e))?;
        // SAFETY: `spawned.process` is a valid, currently-open handle.
        let exit_code = unsafe { rusty_win32::process::wait(spawned.process, None) }
            .map_err(|e| map_win32("WaitForSingleObject", e))?;
        // SAFETY: both handles are valid and each closed exactly once,
        // after every use above.
        unsafe {
            let _ = rusty_win32::handle::close(spawned.process);
            let _ = rusty_win32::handle::close(spawned.thread);
        }
        Ok(ExitStatus {
            code: exit_code.unwrap_or(0) as i32,
        })
    }
}

/// Quote/escape `arg` per the standard Windows `CommandLineToArgvW`
/// convention and append it to `out` -- the algorithm every Windows
/// command-line-building API (including `std::process::Command`) has to
/// implement itself, since there is no OS primitive for it:
/// `rusty_win32::process`'s own module docs explicitly document
/// command-line construction as the caller's responsibility.
#[cfg(windows)]
fn quote_windows_arg(arg: &str, out: &mut String) {
    let needs_quotes = arg.is_empty() || arg.chars().any(|c| c == ' ' || c == '\t' || c == '"');
    if !needs_quotes {
        out.push_str(arg);
        return;
    }
    out.push('"');
    let mut chars = arg.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            let mut backslashes = 1usize;
            while chars.peek() == Some(&'\\') {
                backslashes += 1;
                chars.next();
            }
            if matches!(chars.peek(), Some('"') | None) {
                for _ in 0..backslashes * 2 {
                    out.push('\\');
                }
            } else {
                for _ in 0..backslashes {
                    out.push('\\');
                }
            }
        } else if c == '"' {
            out.push('\\');
            out.push('"');
        } else {
            out.push(c);
        }
    }
    out.push('"');
}

/// Describes the result of a process termination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitStatus {
    code: i32,
}

impl ExitStatus {
    /// Returns the exit code of the process.
    pub fn code(&self) -> Option<i32> {
        Some(self.code)
    }

    /// Returns true if process exited successfully (code 0).
    pub fn success(&self) -> bool {
        self.code == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_reflects_a_real_nonzero_exit_not_a_hardcoded_success() {
        #[cfg(target_os = "linux")]
        let mut cmd = {
            let mut c = Command::new("/bin/sh");
            c.arg("-c");
            c.arg("exit 1");
            c
        };
        #[cfg(windows)]
        let mut cmd = {
            let mut c = Command::new("cmd.exe");
            c.arg("/c");
            c.arg("exit 1");
            c
        };

        let status = cmd
            .status()
            .expect("status() should spawn a real, guaranteed-to-exist command successfully");
        assert!(
            !status.success(),
            "a command that exits 1 must not report success"
        );
        assert_eq!(status.code(), Some(1));
    }
}
