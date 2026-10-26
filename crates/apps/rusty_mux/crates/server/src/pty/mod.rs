//! A block's PTY and the program running on it. Linux drives the master fd
//! from the reactor; Windows bridges ConPTY's pipes with two threads per
//! block. Both expose the same `spawn`, `Pty` and `Process`.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io;
use std::time::Duration;

use platform::error::{ErrorKind, PlatformError};
use platform::process::{Command, EnvSpec};
use platform::term::WinSize;
use rusty_mux_core::Size;

use crate::Config;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "linux")]
pub(crate) use linux::{Process, Pty, spawn};
#[cfg(windows)]
pub(crate) use windows::{Process, Pty, spawn};

/// How long a closed block's program has to exit after the hang-up before
/// it is killed.
const GRACE: Duration = Duration::from_secs(1);
/// How often `Process::reap` polls for the exit.
const POLL: Duration = Duration::from_millis(10);

/// `config`'s program with this process's environment plus `env`.
fn command(config: &Config, env: &[(&str, OsString)]) -> Command {
    let mut vars: BTreeMap<OsString, OsString> = std::env::vars_os().collect();
    vars.extend(env.iter().map(|(k, v)| (OsString::from(k), v.clone())));
    let mut cmd =
        Command::new(config.program.clone(), config.cwd.clone()).env(EnvSpec::Explicit(vars));
    for arg in &config.args {
        cmd = cmd.arg(arg.clone());
    }
    cmd
}

fn win_size(size: Size) -> WinSize {
    WinSize {
        rows: size.rows(),
        cols: size.cols(),
    }
}

fn to_io(err: PlatformError) -> io::Error {
    let kind = match err.kind {
        ErrorKind::WouldBlock => io::ErrorKind::WouldBlock,
        ErrorKind::Interrupted => io::ErrorKind::Interrupted,
        ErrorKind::NotFound => io::ErrorKind::NotFound,
        ErrorKind::PermissionDenied => io::ErrorKind::PermissionDenied,
        ErrorKind::InvalidInput => io::ErrorKind::InvalidInput,
        ErrorKind::BrokenPipe => io::ErrorKind::BrokenPipe,
        _ => io::ErrorKind::Other,
    };
    io::Error::new(kind, format!("{}: {:?}", err.op, err.os))
}
