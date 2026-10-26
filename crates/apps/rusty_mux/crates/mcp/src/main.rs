//! `rusty-mux-mcp [--socket PATH] [--timeout SECS]`: serves the rusty_mux
//! tools over stdio. The socket defaults to `$RUSTY_MUX_SOCKET` and the
//! timeout to [`rusty_mux_mcp::DEFAULT_TIMEOUT`]; see the library docs.

use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use rusty_mux_ctl::Env;

fn main() -> ExitCode {
    let mut env = Env::from_process();
    let mut timeout = rusty_mux_mcp::DEFAULT_TIMEOUT;
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut rest = args.as_slice();
    loop {
        match rest {
            [] => break,
            [flag, path, tail @ ..] if flag == "--socket" => {
                env.socket = Some(path.into());
                rest = tail;
            }
            [flag, secs, tail @ ..] if flag == "--timeout" => {
                let Ok(secs) = secs.parse() else {
                    return usage();
                };
                timeout = Duration::from_secs(secs);
                rest = tail;
            }
            _ => return usage(),
        }
    }
    let server = match rusty_mux_mcp::server(env, timeout) {
        Ok(server) => server,
        Err(err) => {
            eprintln!("rusty-mux-mcp: {err}");
            return ExitCode::FAILURE;
        }
    };
    match rusty_mcp_server::serve_stdio(Arc::new(server)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("rusty-mux-mcp: {err}");
            ExitCode::FAILURE
        }
    }
}

fn usage() -> ExitCode {
    eprintln!("usage: rusty-mux-mcp [--socket PATH] [--timeout SECS]");
    ExitCode::from(2)
}
