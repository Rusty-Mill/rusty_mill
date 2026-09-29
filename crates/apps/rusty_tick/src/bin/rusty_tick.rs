//! `rusty_tick`: serve the task API.
//!
//! ```text
//! RUSTY_TICK_TOKEN=<16+ chars> rusty_tick [--data-dir DIR] [--addr HOST:PORT] [--allow-remote]
//! ```
//!
//! The token is read from the environment (never an argument, which shows up
//! in process listings). The server speaks plain HTTP, so it refuses a
//! non-loopback address unless `--allow-remote` says it sits behind
//! something that terminates TLS.
//!
//! If `DIR/users.json` exists the server runs for several users instead
//! (ADR-0002): each has tokens of the form `<user key>.<secret>` and a store
//! under `DIR/users/`, and `RUSTY_TICK_TOKEN` must not be set, so nobody
//! believes a shared token is being enforced.

use rusty_tick::backend::Backend;
use rusty_tick::pool::DEFAULT_MAX_OPEN_USERS;
use rusty_tick::server::Server;
use rusty_tick::service::system_clock;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;

struct Options {
    data_dir: PathBuf,
    addr: SocketAddr,
    allow_remote: bool,
}

fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Options, String> {
    let mut options = Options {
        data_dir: PathBuf::from("rusty_tick_data"),
        addr: SocketAddr::from(([127, 0, 0, 1], 8787)),
        allow_remote: false,
    };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--data-dir" => {
                options.data_dir = args.next().ok_or("--data-dir needs a value")?.into();
            }
            "--addr" => {
                let text = args.next().ok_or("--addr needs a value")?;
                options.addr = text.parse().map_err(|e| format!("--addr {text:?}: {e}"))?;
            }
            "--allow-remote" => options.allow_remote = true,
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    Ok(options)
}

/// Single-user or multi-user, by whether `users.json` is in `data_dir`.
fn open_backend(data_dir: &std::path::Path) -> Result<Backend, String> {
    let token = std::env::var("RUSTY_TICK_TOKEN");
    let opening =
        |e: rusty_tick::backend::BackendError| format!("opening {}: {e}", data_dir.display());
    if Backend::is_multi_user(data_dir) {
        if token.is_ok() {
            return Err(format!(
                "{} has users.json, so users have their own tokens; unset RUSTY_TICK_TOKEN",
                data_dir.display()
            ));
        }
        return Backend::multi(data_dir, DEFAULT_MAX_OPEN_USERS, Box::new(system_clock))
            .map_err(opening);
    }
    let token = token.map_err(|_| "set RUSTY_TICK_TOKEN to the API token".to_string())?;
    Backend::single(data_dir, token, system_clock()).map_err(opening)
}

fn run() -> Result<(), String> {
    let options = parse_args(std::env::args().skip(1))?;
    if !options.addr.ip().is_loopback() && !options.allow_remote {
        return Err(format!(
            "{} is not a loopback address; pass --allow-remote to serve plain HTTP there",
            options.addr
        ));
    }
    let backend = open_backend(&options.data_dir)?;
    let server = Server::bind(options.addr, backend)
        .map_err(|e| format!("binding {}: {e}", options.addr))?;
    eprintln!("rusty_tick: listening on http://{}", options.addr);
    server.run().map_err(|e| format!("serving: {e}"))
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("rusty_tick: {message}");
            ExitCode::FAILURE
        }
    }
}
