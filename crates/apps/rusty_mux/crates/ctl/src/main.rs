//! `rusty-mux-ctl`; see the library docs and `rusty-mux-ctl help`.

use std::process::ExitCode;

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
    let socket = std::env::var_os("RUSTY_MUX_SOCKET").map(Into::into);
    let exit = rusty_mux_ctl::run(
        &args,
        socket,
        &mut std::io::stdout(),
        &mut std::io::stderr(),
    );
    ExitCode::from(exit as u8)
}
