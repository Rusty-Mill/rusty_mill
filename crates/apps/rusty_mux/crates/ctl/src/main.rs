//! `rusty-mux-ctl`; see the library docs and `rusty-mux-ctl help`.

use std::process::ExitCode;

use rusty_mux_ctl::Env;
use rusty_mux_ctl::cli::Defaults;
use rusty_uuid::Uuid;

fn main() -> ExitCode {
    let args: Vec<String> = match std::env::args_os()
        .skip(1)
        .map(|a| a.into_string())
        .collect()
    {
        Ok(args) => args,
        Err(arg) => {
            eprintln!("rusty-mux-ctl: argument {arg:?} is not UTF-8");
            return ExitCode::from(rusty_mux_ctl::Exit::Usage as u8);
        }
    };
    let env = Env {
        socket: std::env::var_os("RUSTY_MUX_SOCKET").map(Into::into),
        here: Defaults {
            session: id_var("RUSTY_MUX_SESSION"),
            window: id_var("RUSTY_MUX_WINDOW"),
            block: id_var("RUSTY_MUX_BLOCK"),
        },
    };
    let exit = rusty_mux_ctl::run(&args, &env, &mut std::io::stdout(), &mut std::io::stderr());
    ExitCode::from(exit as u8)
}

/// An id from the environment; a malformed one is ignored, with a warning.
fn id_var(name: &str) -> Option<Uuid> {
    let value = std::env::var(name).ok()?;
    match value.parse() {
        Ok(id) => Some(id),
        Err(_) => {
            eprintln!("rusty-mux-ctl: ignoring ${name}: `{value}` is not an id");
            None
        }
    }
}
