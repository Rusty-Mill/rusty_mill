//! The entry point behind `main`, over injectable streams so the whole
//! path from goal text to exit status is testable in-process.
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

use orch_dispatch::AgentRunner;

use crate::args::Args;
use crate::run::{self, Console, Ended};
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
    let spec = input::parse(goal_json)?;
    let mut console = Stdio {
        interactive: args.interactive,
        stdin: streams.stdin,
        stderr: streams.stderr,
    };
    let summary = run::execute(spec, runner, &mut console, Instant::now)?;
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
        Ended::Finished => EXIT_FINISHED,
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
