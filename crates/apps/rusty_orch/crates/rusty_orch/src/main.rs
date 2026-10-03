//! `rusty_orch run <goal.json>`: see [`rusty_orch::args::USAGE`].

use std::io;
use std::process::ExitCode;
use std::time::Duration;

use rusty_orch::agents::{Agents, AgentsConfig};
use rusty_orch::args::{self, ArgsError};
use rusty_orch::cli::{self, Streams};

/// Local models answer in a few minutes; Codex's default stays its own.
const OLLAMA_TIMEOUT: Duration = Duration::from_secs(180);

fn main() -> ExitCode {
    let cwd = std::env::current_dir().unwrap_or_default();
    let parsed = args::parse(std::env::args().skip(1), |k| std::env::var(k).ok(), cwd);
    let args = match parsed {
        Ok(a) => a,
        Err(ArgsError::Help) => {
            println!("{}", args::USAGE);
            return ExitCode::SUCCESS;
        }
        Err(ArgsError::Usage(m)) => {
            eprintln!("rusty_orch: {m}\n\n{}", args::USAGE);
            return ExitCode::from(2);
        }
    };
    let goal_json = match std::fs::read_to_string(&args.goal) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("rusty_orch: cannot read {}: {e}", args.goal.display());
            return ExitCode::from(1);
        }
    };
    let agents = Agents::new(AgentsConfig {
        ollama_model: args.ollama_model.clone(),
        ollama_timeout: OLLAMA_TIMEOUT,
        codex_repo: args.codex_repo.clone(),
        codex_model: args.codex_model.clone(),
    });
    let mut stdin = io::stdin().lock();
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    let mut streams = Streams {
        stdin: &mut stdin,
        stdout: &mut stdout,
        stderr: &mut stderr,
    };
    match cli::run(&args, &goal_json, agents, &mut streams) {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            let _ = writeln!(streams.stderr, "rusty_orch: {e}");
            ExitCode::from(1)
        }
    }
}
