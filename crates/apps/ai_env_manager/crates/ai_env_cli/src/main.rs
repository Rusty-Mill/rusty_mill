use ai_env_cli::{args, run, Outcome};
use std::env;
use std::io;
use std::path::PathBuf;
use std::process::ExitCode;

fn home() -> Option<PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

fn main() -> ExitCode {
    let argv: Vec<String> = env::args().skip(1).collect();
    let cmd = match args::parse(&argv) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("aienv: {e}\n\n{}", args::USAGE);
            return ExitCode::from(2);
        }
    };
    let Some(home) = home() else {
        eprintln!("aienv: set HOME (or USERPROFILE) so ~/ in the tool catalog can be resolved");
        return ExitCode::from(2);
    };
    match run(&cmd, &home, &mut io::stdout().lock()) {
        Ok(Outcome::Done) => ExitCode::SUCCESS,
        Ok(Outcome::Refused) => ExitCode::from(1),
        Err(e) => {
            eprintln!("aienv: {e}");
            ExitCode::from(1)
        }
    }
}
