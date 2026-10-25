//! `rusty-mux-server <socket>`: serves rusty_mux on a Unix socket. Each new
//! block runs `$SHELL` (or `/bin/sh`) in `$HOME` (or `/`).

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
    let config = Config {
        program: std::env::var_os("SHELL").unwrap_or_else(|| OsString::from("/bin/sh")),
        args: Vec::new(),
        cwd: std::env::var_os("HOME").unwrap_or_else(|| OsString::from("/")),
    };
    let runtime = Builder::new_current_thread().build_local(&LocalOptions::default())?;
    runtime.block_on(async {
        let listener = UnixListener::bind(&socket)?;
        // No authentication in Phase 1: only this user may connect.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
        }
        serve(listener, config).await
    })
}
