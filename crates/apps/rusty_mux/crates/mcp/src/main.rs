//! `rusty-mux-mcp [--socket PATH]`: serves the rusty_mux tools over stdio.
//! The socket defaults to `$RUSTY_MUX_SOCKET`; see the library docs.

use std::process::ExitCode;
use std::sync::Arc;

use rusty_mux_ctl::Env;

fn main() -> ExitCode {
    let mut env = Env::from_process();
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [] => {}
        [flag, path] if flag == "--socket" => env.socket = Some(path.into()),
        _ => {
            eprintln!("usage: rusty-mux-mcp [--socket PATH]");
            return ExitCode::from(2);
        }
    }
    let server = match rusty_mux_mcp::server(env) {
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
