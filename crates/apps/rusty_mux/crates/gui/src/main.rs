//! `rusty-mux-gui [--gpu] [<socket> [block]]`: show a rusty_mux block's window in
//! rusty_term's native window, starting with that block focused. Without a
//! socket it uses `$RUSTY_MUX_SOCKET`, else the default socket (C69,
//! `rusty_mux_proto::endpoint`), starting a server there if none answers
//! (`rusty_mux_client::autostart`). Without a
//! block id it creates a session, window and block first. The window
//! follows the server's layout and shared focus; typing goes to the
//! focused block, whose input lease this client holds. rusty_term's config
//! file sets fonts and colours; `--gpu` draws with the GPU renderer, as
//! it does for rusty_term.

use std::process::ExitCode;

fn main() -> ExitCode {
    // rusty_term reads `--gpu` itself.
    let mut args: Vec<_> = std::env::args_os()
        .skip(1)
        .filter(|a| a != "--gpu")
        .collect();
    if args.len() > 2 {
        eprintln!("usage: rusty-mux-gui [--gpu] [<socket> [block]]");
        return ExitCode::from(2);
    }
    if args.is_empty() {
        match default_socket() {
            Ok(socket) => args.push(socket.into_os_string()),
            Err(err) => {
                eprintln!("rusty-mux-gui: {err}");
                return ExitCode::from(2);
            }
        }
    }
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("rusty-mux-gui: {err}");
            ExitCode::FAILURE
        }
    }
}

/// The socket when none is named: `$RUSTY_MUX_SOCKET`, else the default,
/// on which a server is started if nothing answers (C69).
fn default_socket() -> Result<std::path::PathBuf, String> {
    use rusty_mux_proto::endpoint::{Origin, resolve};
    let (socket, origin) = resolve(None, |name| std::env::var_os(name))?;
    #[cfg(any(target_os = "linux", windows))]
    if origin == Origin::Default {
        rusty_mux_client::autostart::ensure(&socket)?;
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    let _ = (origin, Origin::Default);
    Ok(socket)
}

#[cfg(any(target_os = "linux", windows))]
fn run(args: &[std::ffi::OsString]) -> Result<(), String> {
    use rusty_mux_core::BlockId;
    use rusty_mux_gui::source::attach;

    let socket = std::path::PathBuf::from(&args[0]);
    let block = match args.get(1) {
        Some(id) => {
            let id = id.to_str().ok_or("block id is not UTF-8")?;
            Some(BlockId::from_uuid(
                id.parse().map_err(|e| format!("block id: {e}"))?,
            ))
        }
        None => None,
    };
    let (config, warnings) = rusty_term::config::Config::load(&[]);
    for w in &warnings {
        eprintln!("rusty-mux-gui: {w}");
    }
    let attached = attach(&socket, block)?;
    let events = attached.events;
    rusty_term::gui::run_attached(
        &attached.source,
        attached.first,
        attached.focus,
        &config,
        |remote| events.follow(remote),
    )
    .map_err(|e| e.to_string())
}

#[cfg(not(any(target_os = "linux", windows)))]
fn run(_args: &[std::ffi::OsString]) -> Result<(), String> {
    Err("the client runs on Linux and Windows only".into())
}
