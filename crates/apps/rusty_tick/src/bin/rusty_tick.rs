//! `rusty_tick`: serve the task API.
//!
//! ```text
//! RUSTY_TICK_TOKEN=<16+ chars> rusty_tick [--data-dir DIR] [--addr HOST:PORT] [--web-dir DIR] [--allow-remote]
//! ```
//!
//! `--web-dir` serves the built web UI (`web/dist`) at `/`.
//!
//! The token is read from the environment (never an argument, which shows up
//! in process listings). The server speaks plain HTTP, so it refuses a
//! non-loopback address unless `--allow-remote` says it sits behind
//! something that terminates TLS.

use rusty_tick::api::Api;
use rusty_tick::server::Server;
use rusty_tick::service::{system_clock, Service};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;

struct Options {
    data_dir: PathBuf,
    addr: SocketAddr,
    web_dir: Option<PathBuf>,
    allow_remote: bool,
}

fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Options, String> {
    let mut options = Options {
        data_dir: PathBuf::from("rusty_tick_data"),
        addr: SocketAddr::from(([127, 0, 0, 1], 8787)),
        web_dir: None,
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
            "--web-dir" => {
                options.web_dir = Some(args.next().ok_or("--web-dir needs a value")?.into());
            }
            "--allow-remote" => options.allow_remote = true,
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    Ok(options)
}

fn run() -> Result<(), String> {
    let options = parse_args(std::env::args().skip(1))?;
    if !options.addr.ip().is_loopback() && !options.allow_remote {
        return Err(format!(
            "{} is not a loopback address; pass --allow-remote to serve plain HTTP there",
            options.addr
        ));
    }
    let token = std::env::var("RUSTY_TICK_TOKEN")
        .map_err(|_| "set RUSTY_TICK_TOKEN to the API token".to_string())?;
    let api = Api::new(token)?;
    let service = Service::open(&options.data_dir, system_clock())
        .map_err(|e| format!("opening {}: {e}", options.data_dir.display()))?;
    let mut server = Server::bind(options.addr, api, service)
        .map_err(|e| format!("binding {}: {e}", options.addr))?;
    if let Some(dir) = options.web_dir {
        if !dir.join("index.html").is_file() {
            return Err(format!(
                "{} has no index.html; run `npm run build` in web/",
                dir.display()
            ));
        }
        server = server.with_web_dir(dir);
    }
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
