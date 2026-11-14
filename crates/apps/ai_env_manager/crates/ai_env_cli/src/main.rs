use ai_env_cli::args::{self, Command};
use ai_env_cli::{run, Outcome};
use std::env;
use std::io;
use std::path::PathBuf;
use std::process::ExitCode;

/// Exit status: 0 done, 1 a path was refused, 2 usage, 3 error.
const REFUSED: u8 = 1;
const USAGE: u8 = 2;
const FAILED: u8 = 3;

fn main() -> ExitCode {
    let cmd = match args::from_os(env::args_os().skip(1)).and_then(|a| args::parse(&a)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("aienv: {e}\n\n{}", args::USAGE);
            return ExitCode::from(USAGE);
        }
    };
    // Help needs no home folder.
    let home = if cmd == Command::Help {
        Some(PathBuf::new())
    } else {
        ai_env_app::home_dir()
    };
    let Some(home) = home else {
        eprintln!("aienv: set HOME (or USERPROFILE) so ~/ in the tool catalog can be resolved");
        return ExitCode::from(FAILED);
    };
    match run(
        &cmd,
        &home,
        &mut io::stdout().lock(),
        &mut io::stderr().lock(),
    ) {
        Ok(Outcome::Done) => ExitCode::SUCCESS,
        Ok(Outcome::Refused) => ExitCode::from(REFUSED),
        Err(e) => {
            eprintln!("aienv: {e}");
            ExitCode::from(FAILED)
        }
    }
}
