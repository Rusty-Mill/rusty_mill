//! The erosion profile's report-only time box (D42, D88).
//!
//! Report-only lasts 14 days from go-live or 10 report-only claims tasks,
//! whichever comes first. After that, a new report-only claims open is
//! refused until Nano approves limits from the calibration numbers (card
//! #704 Build 5). Tasks already open keep their frozen profiles.
//!
//! The host keeps an append-only ledger, `<dir>/erosion/report-only.log`,
//! one `<unix_ms> <task id>` line per report-only claims open, written
//! before the open dispatches. Go-live is the first line's time. The count
//! is the distinct ledger task ids whose own log proves a report-only
//! claims open: it starts with a claims `TaskOpened` whose frozen profile
//! set runs the erosion profile report-only. A line whose open failed, or
//! whose task id was opened some other way, starts the clock but never
//! counts. A missing ledger means report-only has not gone live.
//!
//! The ledger's exclusive lock is held from the check until the open has
//! published its task log ([`Admission`]), so a racing report-only open
//! always sees the earlier one counted and cannot overshoot.

use rusty_bbp::{log_path_for, Event, Mode, TaskId, Time};
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

/// Why report-only is over, or `None` while it still runs. `counted` says
/// whether a ledger task id is a proven report-only open; each id is asked
/// once, and its error ends the check.
pub fn expiry(
    entries: &[Entry],
    mut counted: impl FnMut(&TaskId) -> Result<bool, String>,
    now: Time,
) -> Result<Option<String>, String> {
    let Some(first) = entries.iter().map(|e| e.at.0).min() else {
        return Ok(None);
    };
    let ids: BTreeSet<&TaskId> = entries.iter().map(|e| &e.task).collect();
    let mut opened = 0;
    for id in ids {
        opened += usize::from(counted(id)?);
    }
    if opened >= MAX_TASKS {
        return Ok(Some(format!("{opened} report-only tasks have opened")));
    }
    if now.0.saturating_sub(first) >= WINDOW_MS {
        return Ok(Some(
            "14 days have passed since report-only went live".into(),
        ));
    }
    Ok(None)
}

/// Whether `task`'s log proves a report-only claims open: its first event
/// is a claims `TaskOpened` whose frozen profile set (checked against the
/// digest) runs the erosion profile report-only. No log, an empty one or a
/// partial first line is `false`; an unreadable log or profile set is an
/// error, so the window fails closed.
pub fn report_only_open(dir: &Path, task: &TaskId) -> Result<bool, String> {
    let log = log_path_for(dir, task);
    let text = match std::fs::read_to_string(&log) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(format!("{}: {e}", log.display())),
    };
    // An empty log, or a first batch cut short by a crash, is not an open
    // (`FsStore::open` truncates it the same way).
    let Some((first, _)) = text.split_once('\n') else {
        return Ok(false);
    };
    let batch: Vec<Event> = rusty_serde::json::from_str(first)
        .map_err(|e| format!("{}: first line: {e}", log.display()))?;
    match batch.first() {
        Some(Event::TaskOpened {
            mode: Mode::Claims,
            profile_digest,
            ..
        }) => Ok(crate::profiles::load(dir, task, *profile_digest)?.erosion_report_only()),
        _ => Ok(false),
    }
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

/// The ledger's exclusive lock, held by an admitted report-only open. Keep
/// it until the open has published its task log, so the next report-only
/// open's count includes this one.
#[must_use = "hold the admission until the open has published its log"]
pub struct Admission(#[allow(dead_code)] File);

/// Record a report-only claims open of `task` at `now`, unless report-only
/// is over: then return why. The line is synced before this returns, so an
/// open that follows is always on the clock, and the returned [`Admission`]
/// keeps the lock for the open itself.
pub fn admit(dir: &Path, task: &TaskId, now: Time) -> Result<Result<Admission, String>, String> {
    if task.0.contains(['\n', '\r']) {
        return Err(format!("task id {:?} has a line break", task.0));
    }
    let mut file = open_locked(dir)?;
    let mut text = String::new();
    file.read_to_string(&mut text)
        .map_err(|e| format!("{}: {e}", path(dir).display()))?;
    let entries = parse(&text)?;
    if let Some(why) = expiry(&entries, |t| report_only_open(dir, t), now)? {
        return Ok(Err(why));
    }
    file.write_all(format!("{} {}\n", now.0, task.0).as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|e| format!("{}: {e}", path(dir).display()))?;
    Ok(Ok(Admission(file)))
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

    fn yes(_: &TaskId) -> Result<bool, String> {
        Ok(true)
    }

    #[test]
    fn an_empty_ledger_has_not_gone_live() {
        assert_eq!(expiry(&[], yes, Time(u64::MAX)), Ok(None));
    }

    #[test]
    fn report_only_ends_at_fourteen_days_from_the_first_line() {
        let e = [entry(1_000, "a"), entry(5_000, "b")];
        assert_eq!(expiry(&e, yes, Time(1_000 + WINDOW_MS - 1)), Ok(None));
        let why = expiry(&e, yes, Time(1_000 + WINDOW_MS)).expect("expiry");
        assert!(why.is_some_and(|w| w.contains("14 days")));
        // A clock that moved backwards does not end it.
        assert_eq!(expiry(&e, yes, Time(0)), Ok(None));
    }

    #[test]
    fn report_only_ends_at_the_tenth_distinct_opened_task() {
        let mut e: Vec<Entry> = (0..9).map(|i| entry(i, &format!("t{i}"))).collect();
        e.push(entry(9, "t0"));
        let mut asked = Vec::new();
        let once = |t: &TaskId| {
            asked.push(t.0.clone());
            Ok(true)
        };
        assert_eq!(expiry(&e, once, Time(10)), Ok(None));
        assert_eq!(asked.len(), 9, "a repeat id is asked and counted once");
        e.push(entry(10, "t9"));
        let why = expiry(&e, yes, Time(11)).expect("expiry");
        assert!(why.is_some_and(|w| w.contains("10 report-only")));
        // A line whose open is not a proven report-only open does not count.
        let not_t9 = |t: &TaskId| Ok(t.0 != "t9");
        assert_eq!(expiry(&e, not_t9, Time(11)), Ok(None));
        // An unreadable task ends the check: the window fails closed.
        let broken = |_: &TaskId| Err("unreadable".to_string());
        assert!(expiry(&e, broken, Time(11)).is_err());
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
        assert!(matches!(admit(&dir, &t, Time(7)), Ok(Ok(_))));
        let text = std::fs::read_to_string(path(&dir)).expect("ledger");
        assert_eq!(text, "7 C1\n");
        let late = admit(&dir, &t, Time(7 + WINDOW_MS)).expect("admit");
        assert!(late.is_err_and(|w| w.contains("14 days")));
        let text = std::fs::read_to_string(path(&dir)).expect("ledger");
        assert_eq!(text, "7 C1\n", "a refused open writes nothing");
        let held = admit(&dir, &TaskId("C2".into()), Time(8)).expect("admit");
        let other = File::open(path(&dir)).expect("ledger");
        assert!(other.try_lock().is_err(), "the admission holds the lock");
        drop(held);
        assert!(other.try_lock().is_ok(), "dropping it releases the lock");
        drop(other);
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
