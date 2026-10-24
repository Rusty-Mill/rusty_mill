//! The entry point behind `main`, over injectable streams so the whole
//! path from goal text to exit status is testable in-process.
//!
//! With `--state <dir>` the plan, board, and ledger are saved there after
//! every dispatcher run and answer round, and a later run on the same
//! directory and goal file continues from them (ADR-0010).
//!
//! Channel discipline: stdout carries the report and nothing else, so
//! `--json` always yields one parseable object. stderr carries two kinds
//! of line: progress, built only from fixed categories, ids, and counts;
//! and, under `--interactive`, the open questions themselves, which are
//! model text by nature. Answers come from stdin. Detailed dispatcher
//! failure text, including adapter and model messages, appears in the
//! report. Startup, input, and I/O errors can still go to stderr.

use std::io::{self, BufRead, Write};
use std::time::Instant;

use orch_core::goal::StopRule;
use orch_dispatch::AgentRunner;

use crate::args::Args;
use crate::millboard::HttpMillboard;
use crate::run::{self, CardHosting, Console, Ended, Hosting, Resume};
use crate::{input, report};

/// Exit status for a finished run.
pub const EXIT_FINISHED: u8 = 0;
/// Exit status when cards wait on unanswered questions.
pub const EXIT_BLOCKED: u8 = 3;
/// Exit status when the dispatcher stopped or the wall clock ran out.
pub const EXIT_FAILED: u8 = 4;

/// The three process streams, borrowed for one run.
pub struct Streams<'a> {
    pub stdin: &'a mut dyn BufRead,
    pub stdout: &'a mut dyn Write,
    pub stderr: &'a mut dyn Write,
}

/// Run `goal_json` with `runner` and print the report. Returns the exit
/// status; an `Err` is a problem the report cannot describe (bad input,
/// bad routing, a dead stream).
pub fn run<R: AgentRunner>(
    args: &Args,
    goal_json: &str,
    runner: R,
    streams: &mut Streams<'_>,
) -> Result<u8, Box<dyn std::error::Error>> {
    run_with(args, goal_json, |_| runner, streams)
}

/// Like [`run`], but the runner is built after the goal file is parsed, so
/// it can be configured with the goal's stop rule (ADR-0011).
pub fn run_with<R: AgentRunner>(
    args: &Args,
    goal_json: &str,
    make_runner: impl FnOnce(StopRule) -> R,
    streams: &mut Streams<'_>,
) -> Result<u8, Box<dyn std::error::Error>> {
    let spec = input::parse(goal_json)?;
    let host = task_host(args, goal_json)?;
    let runner = make_runner(spec.goal.budget().stop());
    let mut console = Stdio {
        interactive: args.interactive,
        stdin: streams.stdin,
        stderr: streams.stderr,
    };
    let mut store = match &args.state {
        Some(dir) => Some(orch_store::Store::open(dir)?),
        None => None,
    };
    let resume = store.as_mut().map(|store| Resume {
        store,
        fingerprint: orch_store::fingerprint(goal_json),
    });
    let summary = run::execute_hosted(spec, runner, &mut console, Instant::now, resume, host)?;
    if args.json {
        writeln!(
            streams.stdout,
            "{}",
            report::json(&summary).to_json_string_pretty()
        )?;
    } else {
        write!(streams.stdout, "{}", report::text(&summary))?;
    }
    Ok(match summary.ended {
        Ended::Finished | Ended::Waiting(_) | Ended::Cancelled => EXIT_FINISHED,
        Ended::Blocked(_) => EXIT_BLOCKED,
        Ended::Failed(_) | Ended::WallClock(_) => EXIT_FAILED,
    })
}

/// stdin for answers, stderr for questions and progress. A non-interactive
/// run never reads. A blank line or end of input means stop.
struct Stdio<'a> {
    interactive: bool,
    stdin: &'a mut dyn BufRead,
    stderr: &'a mut dyn Write,
}

impl Console for Stdio<'_> {
    fn note(&mut self, line: &str) {
        let _ = writeln!(self.stderr, "{line}");
    }

    fn ask(&mut self, question: &str) -> io::Result<Option<String>> {
        if !self.interactive {
            return Ok(None);
        }
        write!(self.stderr, "{question}\nanswer (blank to stop)> ")?;
        self.stderr.flush()?;
        let mut line = String::new();
        let read = self.stdin.read_line(&mut line)?;
        Ok((read > 0 && !line.trim().is_empty()).then_some(line))
    }
}

/// Configuration stays at the I/O composition root; orch-core remains pure.
fn task_host(args: &Args, goal_json: &str) -> Result<Option<Hosting>, Box<dyn std::error::Error>> {
    let root: rusty_serde::json::Value = rusty_serde::json::from_str(goal_json)?;
    let config: Option<orch_bbp::Config> = match &args.bbp {
        Some(file) => Some(rusty_serde::json::from_str(&std::fs::read_to_string(
            file,
        )?)?),
        None => root
            .get("bbp")
            .filter(|v| !v.is_null())
            .map(|v| {
                rusty_serde::json::to_string(v).and_then(|json| rusty_serde::json::from_str(&json))
            })
            .transpose()?,
    };
    let Some(config) = config else {
        if args.cancel.is_some() {
            return Err("--cancel requires BBP task hosting (--bbp or the goal's bbp)".into());
        }
        if args.card.is_some() {
            return Err("--card requires BBP task hosting (--bbp or the goal's bbp)".into());
        }
        return Ok(None);
    };
    if args.state.is_none() {
        return Err("BBP task hosting requires --state for recovery".into());
    }
    let spec = input::parse(goal_json)?;
    let lines = |values: &[orch_core::Text]| {
        values
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n- ")
    };
    let context = format!(
        "Goal: {}\nDONE WHEN:\n- {}\nConstraints:\n- {}\nIn scope:\n- {}\nOut of scope:\n- {}\nGoal references:\n- {}",
        spec.goal.outcome(), lines(spec.goal.done_when()), lines(spec.goal.constraints()),
        lines(spec.goal.in_scope()), lines(spec.goal.out_of_scope()), lines(spec.goal.refs())
    );
    let namespace = rusty_bbp::Sha256::of(goal_json.as_bytes()).hex();
    let card = match &args.card {
        Some(card) => Some(CardHosting {
            card: card.card.clone(),
            namespace: namespace.clone(),
            board: Box::new(HttpMillboard::new(card.board.clone())?),
        }),
        None => None,
    };
    let host = orch_bbp::BbpHost::new(config, namespace, context)?;
    Ok(Some(Hosting {
        host: Box::new(host),
        card,
        cancel: args.cancel.clone(),
    }))
}
