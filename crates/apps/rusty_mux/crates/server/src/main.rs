//! `rusty-mux-server <socket>`: serves rusty_mux on a Unix socket. Each new
//! block runs `$SHELL` (or `/bin/sh`) in `$HOME` (or `/`).
//!
//! On Windows (10 1803 or later, for AF_UNIX) each block runs `%COMSPEC%`
//! (or `cmd.exe`) in `%USERPROFILE%` (or `C:\`) on a pseudo console. Put
//! the socket under `%LOCALAPPDATA%`, for example
//! `rusty-mux-server %LOCALAPPDATA%\rusty-mux.sock`; the server limits the
//! file to the current user.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use rusty_mux_server::{Config, serve};
use rusty_tokio::io::UnixListener;
use rusty_tokio::{Builder, LocalOptions};

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let (Some(socket), None) = (args.next(), args.next()) else {
        eprintln!("usage: rusty-mux-server <socket>");
        return ExitCode::from(2);
    };
    match run(PathBuf::from(socket)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("rusty-mux-server: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run(socket: PathBuf) -> std::io::Result<()> {
    let config = default_config();
    let runtime = Builder::new_current_thread().build_local(&LocalOptions::default())?;
    runtime.block_on(async {
        let listener = UnixListener::bind(&socket)?;
        // No authentication in Phase 1: only this user may connect.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
        }
        // The Windows form of 0600: one entry, full control for this user.
        #[cfg(windows)]
        platform_windows::fs::restrict_to_owner(&socket).map_err(|e| {
            std::io::Error::other(format!("restricting {}: {e:?}", socket.display()))
        })?;
        serve(listener, config).await
    })
}

#[cfg(not(windows))]
fn default_config() -> Config {
    Config {
        program: std::env::var_os("SHELL").unwrap_or_else(|| OsString::from("/bin/sh")),
        args: Vec::new(),
        cwd: std::env::var_os("HOME").unwrap_or_else(|| OsString::from("/")),
    }
}

#[cfg(windows)]
fn default_config() -> Config {
    Config {
        program: std::env::var_os("COMSPEC").unwrap_or_else(|| OsString::from("cmd.exe")),
        args: Vec::new(),
        cwd: std::env::var_os("USERPROFILE").unwrap_or_else(|| OsString::from("C:\\")),
    }
}
