//! `rusty-mux-ctl`; see the library docs and `rusty-mux-ctl help`.

use std::process::ExitCode;

use rusty_mux_ctl::Env;

fn main() -> ExitCode {
    let args: Vec<String> = match std::env::args_os()
        .skip(1)
        .map(|a| a.into_string())
        .collect()
    {
        Ok(args) => args,
        Err(arg) => {
            eprintln!("rusty-mux-ctl: argument {arg:?} is not UTF-8");
            return ExitCode::from(rusty_mux_ctl::Exit::Usage.code());
        }
    };
    let env = Env::from_process();
    let exit = rusty_mux_ctl::run_process(&args, &env);
    ExitCode::from(exit.code())
}
