//! The erosion profile's report-only time box (D42, D88).
//!
//! Report-only lasts 14 days from go-live or 10 report-only claims tasks,
//! whichever comes first. After that, a new report-only claims open is
//! refused until Nano approves limits from the calibration numbers (card
//! #704 Build 5). Tasks already open keep their frozen profiles.
//!
//! The host keeps an append-only ledger, `<dir>/erosion/report-only.log`,
//! one `<unix_ms> <task id>` line per report-only claims open, written
//! before the open dispatches. Go-live is the first line's time; the count
//! is the distinct task ids that also have a task log, so a line whose open
//! then failed only starts the clock, never adds to the count. A missing
//! ledger means report-only has not gone live. The check and the append run
//! under an exclusive lock on the ledger, so racing opens cannot overshoot.

use rusty_bbp::{log_path_for, TaskId, Time};
use std::collections::BTreeSet;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Report-only lasts this long from go-live (D42): 14 days.
pub const WINDOW_MS: u64 = 14 * 24 * 60 * 60 * 1000;
/// Report-only covers at most this many claims tasks (D42).
pub const MAX_TASKS: usize = 10;

/// `<dir>/erosion/report-only.log`.
pub fn path(dir: &Path) -> PathBuf {
    dir.join("erosion").join("report-only.log")
}

/// One ledger line: when a report-only claims open was recorded, and for
/// which task.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub at: Time,
    pub task: TaskId,
}

/// Why report-only is over, or `None` while it still runs. Pure: `opened`
/// counts only the entries whose task has a log.
pub fn expiry(entries: &[Entry], opened: impl Fn(&TaskId) -> bool, now: Time) -> Option<String> {
    let first = entries.iter().map(|e| e.at.0).min()?;
    let tasks: BTreeSet<&str> = entries
        .iter()
        .filter(|e| opened(&e.task))
        .map(|e| e.task.0.as_str())
        .collect();
    if tasks.len() >= MAX_TASKS {
        return Some(format!("{} report-only tasks have opened", tasks.len()));
    }
    if now.0.saturating_sub(first) >= WINDOW_MS {
        return Some("14 days have passed since report-only went live".into());
    }
    None
}

/// Parse the ledger. A malformed line is an error, so a damaged ledger
/// refuses opens rather than restarting the clock.
pub fn parse(text: &str) -> Result<Vec<Entry>, String> {
    text.lines()
        .enumerate()
        .map(|(i, line)| {
            let bad = || format!("{} line {}: {line:?}", "erosion ledger", i + 1);
            let (at, task) = line.split_once(' ').ok_or_else(bad)?;
            let at = at.parse::<u64>().map_err(|_| bad())?;
            match task.is_empty() {
                true => Err(bad()),
                false => Ok(Entry {
                    at: Time(at),
                    task: TaskId(task.into()),
                }),
            }
        })
        .collect()
}

/// Record a report-only claims open of `task` at `now`, unless report-only
/// is over: then return why. Runs under the ledger's exclusive lock; the
/// line is synced before this returns `Ok`, so an open that follows is
/// always counted.
pub fn admit(dir: &Path, task: &TaskId, now: Time) -> Result<Result<(), String>, String> {
    if task.0.contains(['\n', '\r']) {
        return Err(format!("task id {:?} has a line break", task.0));
    }
    let mut file = open_locked(dir)?;
    let mut text = String::new();
    file.read_to_string(&mut text)
        .map_err(|e| format!("{}: {e}", path(dir).display()))?;
    let entries = parse(&text)?;
    if let Some(why) = expiry(&entries, |t| log_path_for(dir, t).exists(), now) {
        return Ok(Err(why));
    }
    file.write_all(format!("{} {}\n", now.0, task.0).as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|e| format!("{}: {e}", path(dir).display()))?;
    Ok(Ok(()))
}

fn open_locked(dir: &Path) -> Result<File, String> {
    let p = path(dir);
    let io = |e: std::io::Error| format!("{}: {e}", p.display());
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).map_err(io)?;
    }
    let file = OpenOptions::new()
        .read(true)
        .append(true)
        .create(true)
        .open(&p)
        .map_err(io)?;
    file.lock().map_err(io)?;
    Ok(file)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    fn entry(at: u64, task: &str) -> Entry {
        Entry {
            at: Time(at),
            task: TaskId(task.into()),
        }
    }

    #[test]
    fn an_empty_ledger_has_not_gone_live() {
        assert_eq!(expiry(&[], |_| true, Time(u64::MAX)), None);
    }

    #[test]
    fn report_only_ends_at_fourteen_days_from_the_first_line() {
        let e = [entry(1_000, "a"), entry(5_000, "b")];
        assert_eq!(expiry(&e, |_| true, Time(1_000 + WINDOW_MS - 1)), None);
        let why = expiry(&e, |_| true, Time(1_000 + WINDOW_MS));
        assert!(why.is_some_and(|w| w.contains("14 days")));
        // A clock that moved backwards does not end it.
        assert_eq!(expiry(&e, |_| true, Time(0)), None);
    }

    #[test]
    fn report_only_ends_at_the_tenth_distinct_opened_task() {
        let mut e: Vec<Entry> = (0..9).map(|i| entry(i, &format!("t{i}"))).collect();
        e.push(entry(9, "t0"));
        assert_eq!(
            expiry(&e, |_| true, Time(10)),
            None,
            "a repeat id counts once"
        );
        e.push(entry(10, "t9"));
        let why = expiry(&e, |_| true, Time(11));
        assert!(why.is_some_and(|w| w.contains("10 report-only")));
        // A line whose open failed has no task log: it does not count.
        assert_eq!(expiry(&e, |t| t.0 != "t9", Time(11)), None);
    }

    #[test]
    fn the_ledger_parses_and_a_damaged_line_is_an_error() {
        let ok = parse("12 a\n34 task with spaces\n").expect("parse");
        assert_eq!(ok, vec![entry(12, "a"), entry(34, "task with spaces")]);
        for bad in ["x a\n", "12\n", "12 \n", "-1 a\n"] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn admit_appends_until_report_only_is_over() {
        let dir = std::env::temp_dir().join(format!("erosion_window_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let t = TaskId("C1".into());
        assert_eq!(admit(&dir, &t, Time(7)), Ok(Ok(())));
        let text = std::fs::read_to_string(path(&dir)).expect("ledger");
        assert_eq!(text, "7 C1\n");
        let late = admit(&dir, &t, Time(7 + WINDOW_MS)).expect("admit");
        assert!(late.is_err_and(|w| w.contains("14 days")));
        let text = std::fs::read_to_string(path(&dir)).expect("ledger");
        assert_eq!(text, "7 C1\n", "a refused open writes nothing");
        let broken = TaskId("a\nb".into());
        assert!(admit(&dir, &broken, Time(8)).is_err());
        std::fs::write(path(&dir), "garbage\n").expect("write");
        assert!(
            admit(&dir, &t, Time(8)).is_err(),
            "a damaged ledger refuses"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
