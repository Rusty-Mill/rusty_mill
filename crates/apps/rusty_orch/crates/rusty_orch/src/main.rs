//! `rusty_orch run <goal.json>`: see [`rusty_orch::args::USAGE`].

use std::io;
use std::process::ExitCode;
use std::time::Duration;

use rusty_orch::agents::{Agents, AgentsConfig};
use rusty_orch::args::{self, ArgsError};
use rusty_orch::cli::{self, Streams};

/// Local models answer in a few minutes; Codex's and Claude's defaults stay
/// their own.
const OLLAMA_TIMEOUT: Duration = Duration::from_secs(180);

fn main() -> ExitCode {
    let mut argv = std::env::args().skip(1);
    match argv.next().as_deref() {
        Some("inspect") => return inspect(argv),
        Some("maintain") => return maintain(argv),
        _ => {}
    }
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
    let agents = |stop| {
        Agents::new(AgentsConfig {
            ollama_model: args.ollama_model.clone(),
            ollama_timeout: OLLAMA_TIMEOUT,
            repo: args.repo.clone(),
            codex_model: args.codex_model.clone(),
            claude_model: args.claude_model.clone(),
            stop,
        })
    };
    let mut stdin = io::stdin().lock();
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    let mut streams = Streams {
        stdin: &mut stdin,
        stdout: &mut stdout,
        stderr: &mut stderr,
    };
    match cli::run_with(&args, &goal_json, agents, &mut streams) {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            let _ = writeln!(streams.stderr, "rusty_orch: {e}");
            ExitCode::from(1)
        }
    }
}

fn inspect(argv: impl IntoIterator<Item = String>) -> ExitCode {
    use rusty_orch::inspect;
    let cwd = match std::env::current_dir() {
        Ok(cwd) => cwd,
        Err(error) => {
            eprintln!("rusty_orch: {error}");
            return ExitCode::from(1);
        }
    };
    let options = match inspect::options(argv, cwd) {
        Ok(repo) => repo,
        Err(ArgsError::Help) => {
            println!("{}", inspect::USAGE);
            return ExitCode::SUCCESS;
        }
        Err(ArgsError::Usage(message)) => {
            eprintln!("rusty_orch: {message}\n{}", inspect::USAGE);
            return ExitCode::from(2);
        }
    };
    let diagnostics = std::env::var_os("RUSTY_ORCH_METADATA_DIAGNOSTICS").as_deref()
        == Some(std::ffi::OsStr::new("1"));
    match inspect::inspect_with_impact_diagnostics(
        &options.repo,
        options.impact.as_deref(),
        &orch_cli::StdCommand,
    ) {
        Ok(output) => {
            println!("{}", output.report);
            if diagnostics && !output.diagnostics.is_empty() {
                eprint!("{}", output.diagnostics);
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("rusty_orch: {error}");
            ExitCode::from(1)
        }
    }
}

fn maintain(argv: impl IntoIterator<Item = String>) -> ExitCode {
    use rusty_orch::maintenance;
    let cwd = match std::env::current_dir() {
        Ok(cwd) => cwd,
        Err(error) => {
            eprintln!("rusty_orch: {error}");
            return ExitCode::from(1);
        }
    };
    let options = match maintenance::parse(argv, cwd) {
        Ok(options) => options,
        Err(ArgsError::Help) => {
            println!("{}", maintenance::USAGE);
            return ExitCode::SUCCESS;
        }
        Err(ArgsError::Usage(message)) => {
            eprintln!("{message}\n{}", maintenance::USAGE);
            return ExitCode::from(2);
        }
    };
    match maintenance::run(&options) {
        Ok(report) => {
            println!("{report}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("rusty_orch: {error}");
            ExitCode::from(1)
        }
    }
}
