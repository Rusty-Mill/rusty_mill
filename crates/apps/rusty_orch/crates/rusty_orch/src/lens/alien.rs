//! `rusty_orch lens alien report`: the alien lens's shadow numbers (#708,
//! design D75 §8), read from the orch state directory.

use std::fmt::Write as _;
use std::path::PathBuf;

use orch_lens::alien::{summarize, Bar, ProbeRow, BAR_HUNTS};
use orch_store::Store;

use crate::args::ArgsError;

pub const USAGE: &str = "usage: rusty_orch lens alien report --state <orch state dir> [--task <task>]\n\nPrint the alien lens's probe rows and its shadow numbers: hunts, regressions no ordinary\nlens named, and whether the success bar (1 such regression per 20 hunts) is met yet.";

/// What to report.
#[derive(Debug, PartialEq, Eq)]
pub struct Options {
    pub state: PathBuf,
    pub task: Option<String>,
}

/// Parse `alien report --state <dir> [--task <task>]`.
pub fn options(argv: impl IntoIterator<Item = String>) -> Result<Options, ArgsError> {
    let mut args = argv.into_iter();
    if args.next().as_deref() != Some("alien") {
        return Err(ArgsError::Help);
    }
    match args.next().as_deref() {
        Some("report") => {}
        Some("-h" | "--help") | None => return Err(ArgsError::Help),
        Some(other) => return Err(ArgsError::Usage(format!("unknown alien command {other:?}"))),
    }
    let (mut state, mut task) = (None, None);
    while let Some(arg) = args.next() {
        let slot = match arg.as_str() {
            "-h" | "--help" => return Err(ArgsError::Help),
            "--state" => &mut state,
            "--task" => &mut task,
            _ => return Err(ArgsError::Usage(format!("unexpected argument {arg:?}"))),
        };
        if slot.is_some() {
            return Err(ArgsError::Usage(format!("duplicate {arg}")));
        }
        let value = args
            .next()
            .filter(|v| !v.is_empty() && !v.starts_with('-'))
            .ok_or_else(|| ArgsError::Usage(format!("{arg} needs a value")))?;
        *slot = Some(value);
    }
    Ok(Options {
        state: state
            .map(PathBuf::from)
            .ok_or_else(|| ArgsError::Usage("--state is required".into()))?,
        task,
    })
}

/// Read the probe rows and render them.
pub fn run(o: &Options) -> Result<String, String> {
    let store = Store::open(&o.state).map_err(|e| e.to_string())?;
    let rows = store.probes(o.task.as_deref()).map_err(|e| e.to_string())?;
    Ok(render(&rows))
}

/// One line per probe, then the totals and the bar.
pub fn render(rows: &[ProbeRow]) -> String {
    let mut out = String::new();
    for r in rows {
        let _ = writeln!(
            out,
            "{} claim {} candidate {}: {} {} {}{} ({} runs, {} tokens)",
            r.task,
            r.claim,
            r.candidate,
            r.mechanism,
            r.site,
            r.outcome.name(),
            if r.novel { ", novel" } else { "" },
            r.runs,
            r.tokens
        );
    }
    let s = summarize(rows);
    let _ = writeln!(
        out,
        "{} hunts, {} probes: {} regressions ({} novel), {} pre-existing, {} errors; {} runs, {} tokens",
        s.hunts, s.probes, s.regressions, s.novel, s.pre_existing, s.errors, s.runs, s.tokens
    );
    let bar = match s.bar() {
        Bar::TooFew => format!("bar: not judged before {BAR_HUNTS} hunts"),
        Bar::Met => "bar: met (design v2 may be proposed)".to_owned(),
        Bar::NotMet => "bar: not met".to_owned(),
    };
    let _ = writeln!(out, "{bar}");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use orch_lens::alien::Outcome;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn report_options_parse() {
        assert_eq!(
            options(args(&["alien", "report", "--state", "/s", "--task", "t"])),
            Ok(Options {
                state: PathBuf::from("/s"),
                task: Some("t".to_owned()),
            })
        );
        assert_eq!(options(args(&["alien"])), Err(ArgsError::Help));
        for bad in [
            &["alien", "report"][..],
            &["alien", "report", "--state"],
            &["alien", "report", "--state", "/s", "--state", "/t"],
            &["alien", "report", "--state", "/s", "x"],
            &["alien", "hunt"],
        ] {
            assert!(
                matches!(options(args(bad)), Err(ArgsError::Usage(_))),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn render_lists_rows_then_totals_and_bar() {
        let row = ProbeRow {
            task: "t".to_owned(),
            claim: 7,
            candidate: 3,
            site: "store::write".to_owned(),
            mechanism: "rs-atomic".to_owned(),
            novel: true,
            outcome: Outcome::Regression,
            runs: 2,
            tokens: 900,
            probe_sha256: "00".to_owned(),
        };
        assert_eq!(
            render(&[row]),
            "t claim 7 candidate 3: rs-atomic store::write regression, novel (2 runs, 900 tokens)\n\
             1 hunts, 1 probes: 1 regressions (1 novel), 0 pre-existing, 0 errors; 2 runs, 900 tokens\n\
             bar: not judged before 20 hunts\n"
        );
        assert!(render(&[]).starts_with("0 hunts"));
    }
}
