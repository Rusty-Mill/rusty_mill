//! Shadow records kept across processes, and `bbp shadow`: how often an
//! event-driven rule would have picked a different role than the real grant.
//!
//! The core returns the records from every dispatch and logs nothing. Every
//! host process that opens a driver through [`crate::open_driver`] appends
//! them to `<dir>/shadow/<sha256(task)>.jsonl`, one JSON record per line,
//! deduped on `rev`: a grant is one logged `turn_granted`, so a `rev` is
//! written at most once. Advisory: a failure to log is reported on stderr and
//! never fails the command that made the grant.

use crate::moderator::task_dir;
use rusty_bbp::*;
use rusty_serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

/// The file holding `task`'s shadow records.
pub fn log_path(dir: &Path, task: &TaskId) -> PathBuf {
    task_dir(dir, "shadow", task).with_extension("jsonl")
}

/// The sink [`crate::open_driver`] installs: append, and report a failure.
pub fn sink(dir: PathBuf, task: TaskId) -> ShadowSink {
    Box::new(move |records| {
        if let Err(e) = append(&dir, &task, records) {
            eprintln!("bbp: shadow log not written: {e}");
        }
    })
}

/// What a log holds: its records and the number of lines that did not parse
/// (a torn last line after a crash, or a foreign line). Never dropped silently.
#[derive(Debug, Default)]
pub struct Logged {
    pub records: Vec<Shadow>,
    pub unreadable: usize,
}

/// The log's text; an absent file is an empty log.
fn text(dir: &Path, task: &TaskId) -> Result<String, String> {
    match fs::read_to_string(log_path(dir, task)) {
        Ok(t) => Ok(t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e.to_string()),
    }
}

fn parse(text: &str) -> Logged {
    let mut out = Logged::default();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        match rusty_serde::json::from_str::<Shadow>(line) {
            Ok(s) => out.records.push(s),
            Err(_) => out.unreadable += 1,
        }
    }
    out
}

/// Read `task`'s log; an absent file is an empty log.
pub fn read(dir: &Path, task: &TaskId) -> Result<Logged, String> {
    Ok(parse(&text(dir, task)?))
}

/// Append the records whose `rev` is not already in the log. Returns how many
/// were written. A line that does not parse cannot hide a later duplicate: the
/// dedupe reads every line it can. If the file ends without a newline (a write
/// cut short), the new records start on a fresh line, so the torn fragment stays
/// one unreadable line and never swallows a record.
pub fn append(dir: &Path, task: &TaskId, records: &[Shadow]) -> Result<usize, String> {
    let existing = text(dir, task)?;
    let mut seen: BTreeSet<u64> = parse(&existing).records.iter().map(|s| s.rev.0).collect();
    let mut lines = String::new();
    if !existing.is_empty() && !existing.ends_with('\n') {
        lines.push('\n');
    }
    let mut written = 0;
    for s in records {
        if !seen.insert(s.rev.0) {
            continue;
        }
        lines.push_str(&rusty_serde::json::to_string(s).map_err(|e| e.to_string())?);
        lines.push('\n');
        written += 1;
    }
    if written == 0 {
        return Ok(0);
    }
    let path = log_path(dir, task);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| e.to_string())?;
    f.write_all(lines.as_bytes()).map_err(|e| e.to_string())?;
    Ok(written)
}

/// Counts for one task state.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Row {
    pub agree: u64,
    pub fallback: u64,
    pub disagree: u64,
    pub explicit: u64,
}

/// What `bbp shadow` prints.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Report {
    pub task: String,
    pub records: u64,
    pub unreadable_lines: u64,
    pub total: Row,
    /// By the state the grant was made in.
    pub by_state: BTreeMap<String, Row>,
    /// By the wake rules the record was computed under (`WAKE_RULES`). Records
    /// from different rules must not be summed: read one key, or run on a fresh
    /// store. A record written before the field existed counts under "1".
    pub by_rules: BTreeMap<String, BTreeMap<String, Row>>,
}

/// Tally `task`'s shadow log.
pub fn of(dir: &Path, task: &TaskId) -> Result<Report, String> {
    let logged = read(dir, task)?;
    let mut report = Report {
        task: task.0.clone(),
        records: logged.records.len() as u64,
        unreadable_lines: logged.unreadable as u64,
        total: Row::default(),
        by_state: BTreeMap::new(),
        by_rules: BTreeMap::new(),
    };
    for s in &logged.records {
        let state = format!("{:?}", s.state);
        let by_rules = report.by_rules.entry(s.rules.to_string()).or_default();
        let by_rules_row = by_rules.entry(state.clone()).or_default();
        let row = report.by_state.entry(state).or_default();
        for r in [row, by_rules_row, &mut report.total] {
            match s.comparison() {
                Some(Comparison::Agree) => r.agree += 1,
                Some(Comparison::Fallback) => r.fallback += 1,
                Some(Comparison::Disagree) => r.disagree += 1,
                None => r.explicit += 1,
            }
        }
    }
    Ok(report)
}

/// [`of`] as one line of JSON.
pub fn json(dir: &Path, task: &TaskId) -> Result<String, String> {
    rusty_serde::json::to_string(&of(dir, task)?).map_err(|e| e.to_string())
}
