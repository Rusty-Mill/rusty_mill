//! `rusty_orch run <goal.json>`: see [`rusty_orch::args::USAGE`].

use std::io::{self, BufRead, Write};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use rusty_orch::agents::{Agents, AgentsConfig};
use rusty_orch::args::{self, ArgsError};
use rusty_orch::run::{self, Console, Ended};
use rusty_orch::{input, report};

/// Local models answer in a few minutes; Codex's default stays its own.
const OLLAMA_TIMEOUT: Duration = Duration::from_secs(180);

/// stdin for answers, stderr for progress. Non-interactive runs never read.
struct Stdio {
    interactive: bool,
}

impl Console for Stdio {
    fn note(&mut self, line: &str) {
        eprintln!("{line}");
    }

    fn ask(&mut self, question: &str) -> io::Result<Option<String>> {
        if !self.interactive {
            return Ok(None);
        }
        let mut out = io::stdout().lock();
        write!(out, "{question}\nanswer (blank to stop)> ")?;
        out.flush()?;
        let mut line = String::new();
        let read = io::stdin().lock().read_line(&mut line)?;
        Ok((read > 0 && !line.trim().is_empty()).then_some(line))
    }
}

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
    match run(args) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("rusty_orch: {e}");
            ExitCode::from(1)
        }
    }
}

fn run(args: args::Args) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let json = std::fs::read_to_string(&args.goal)
        .map_err(|e| format!("cannot read {}: {e}", args.goal.display()))?;
    let spec = input::parse(&json)?;
    let agents = Agents::new(AgentsConfig {
        ollama_model: args.ollama_model,
        ollama_timeout: OLLAMA_TIMEOUT,
        codex_repo: args.codex_repo,
        codex_model: args.codex_model,
    });
    let mut console = Stdio {
        interactive: args.interactive,
    };
    let summary = run::execute(spec, agents, &mut console, Instant::now)?;
    if args.json {
        println!("{}", report::json(&summary).to_json_string_pretty());
    } else {
        print!("{}", report::text(&summary));
    }
    Ok(match summary.ended {
        Ended::Finished => ExitCode::SUCCESS,
        Ended::Blocked(_) => ExitCode::from(3),
        Ended::Failed(_) | Ended::WallClock(_) => ExitCode::from(4),
    })
}
