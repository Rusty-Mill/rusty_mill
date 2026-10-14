//! `mill-runner`: works Mill Board cards through stages.
//!
//! ```text
//! mill-runner --board http://127.0.0.1:8787 --stages stages.json --repo ~/src/app --work ~/mill-work
//! ```
//! Each stage names a column, a board role and a command. The command runs
//! without a shell, sandboxed, in a git worktree of the card's own branch;
//! the card moves on when it succeeds. The runner also acts as the sandbox
//! helper (`mill-runner __sandbox`), started by itself.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::Parser;
use rusty_mill_runner::config;
use rusty_mill_runner::git_work::GitWork;
use rusty_mill_runner::http_board::HttpBoard;
use rusty_mill_runner::ports::BoardApi;
use rusty_mill_runner::runner::{Runner, Scope};

/// Mill Board runner.
#[derive(Parser)]
struct Cli {
    /// Board base URL.
    #[arg(long, env = "MILL_BOARD_URL", default_value = "http://127.0.0.1:8787")]
    board: String,
    /// Bearer token for the board.
    #[arg(long, env = "MILL_BOARD_TOKEN", hide_env_values = true)]
    token: Option<String>,
    /// Name recorded on claims and comments.
    #[arg(long, default_value = "mill-runner")]
    actor: String,
    /// Stage file (JSON array).
    #[arg(long)]
    stages: PathBuf,
    /// The repository cards are worked in.
    #[arg(long)]
    repo: PathBuf,
    /// Directory for worktrees and scratch space (outside the repository).
    #[arg(long)]
    work: PathBuf,
    /// Branch new card branches start from.
    #[arg(long, default_value = "main")]
    base: String,
    /// Seconds between passes.
    #[arg(long, default_value_t = 5)]
    interval_secs: u64,
    /// Cards worked at once.
    #[arg(long, default_value_t = 1)]
    parallel: usize,
    /// Extra directory commands may read (repeatable), e.g. a toolchain or ~/.claude.
    #[arg(long = "read-root")]
    read_roots: Vec<PathBuf>,
    /// Environment variable passed to commands (repeatable). PATH is always passed.
    #[arg(long = "env")]
    env: Vec<String>,
    /// Work only this project's cards, and discard any stage whose work changes files outside
    /// the project's paths (see the board's `board_projects`). Required when the board has
    /// several projects.
    #[arg(long, env = "MILL_PROJECT")]
    project: Option<String>,
    /// Repository paths every project may change besides its own (repeatable): change
    /// fragments and the lockfile by default.
    #[arg(long = "shared-path", default_values_t = ["changes".to_owned(), "Cargo.lock".to_owned()])]
    shared: Vec<String>,
    /// Do one pass and exit.
    #[arg(long)]
    once: bool,
}

fn run(cli: Cli) -> Result<(), String> {
    let stages = config::parse(
        &std::fs::read_to_string(&cli.stages)
            .map_err(|e| format!("{}: {e}", cli.stages.display()))?,
    )?;
    let helper = std::env::current_exe().map_err(|e| e.to_string())?;
    let work = GitWork::new(&cli.repo, &cli.work, &cli.base, helper, cli.read_roots)?;
    let board = HttpBoard::new(&cli.board, cli.token, &cli.actor).map_err(|e| e.to_string())?;
    let env = std::iter::once("PATH".to_owned())
        .chain(cli.env)
        .filter_map(|name| std::env::var(&name).ok().map(|v| (name, v)))
        .collect();
    let scope = match &cli.project {
        Some(name) => {
            let projects = board.projects().map_err(|e| e.to_string())?;
            Some(Scope::new(name, projects, cli.shared)?)
        }
        None => None,
    };
    let mut runner = Runner::new(board, work, stages, &cli.actor, cli.parallel, env);
    if let Some(scope) = scope {
        runner = runner.with_scope(scope);
    }
    runner.validate()?;
    tracing::info!(board = %cli.board, "runner started");
    loop {
        match runner.tick() {
            Ok(steps) => steps.iter().for_each(|s| tracing::info!(?s, "step")),
            Err(e) => tracing::warn!(error = %e, "pass failed; will retry"),
        }
        if cli.once {
            return Ok(());
        }
        std::thread::sleep(Duration::from_secs(cli.interval_secs.max(1)));
    }
}

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    if args.first().and_then(|a| a.to_str()) == Some("__sandbox") {
        let error = rusty_sandbox::run_helper(&args[1..]);
        eprintln!("mill-runner __sandbox: {error}");
        return ExitCode::from(rusty_sandbox::SETUP_FAILED);
    }
    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .try_init();
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(why) => {
            eprintln!("mill-runner: {why}");
            ExitCode::FAILURE
        }
    }
}
