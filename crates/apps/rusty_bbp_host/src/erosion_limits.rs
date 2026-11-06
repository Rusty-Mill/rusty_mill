//! Nano's approved erosion limits (D115, D118).
//!
//! Only `bbp erosion approve` writes `<dir>/erosion/limits.log`: one JSON
//! line per approval, appended under the file's lock, newest line wins.
//! Each line names where Nano approved it (`source`). The host records that
//! reference and does not verify it: like D37's golden sign-off this is an
//! attestation, not authentication. Anything that runs host commands as the
//! operator can write a line, as it can run `bbp human` (D88's boundary);
//! sandboxed agents and the Agent API can't reach the file.
//!
//! A claims open whose erosion profile enforces limits must name exactly
//! the newest record's numbers. No record, a different pair or a damaged
//! log refuses the open, so the check fails closed.

use rusty_bbp::Time;
use rusty_serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// `<dir>/erosion/limits.log`.
pub fn path(dir: &Path) -> PathBuf {
    dir.join("erosion").join("limits.log")
}

/// One approval.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub at_ms: u64,
    pub concentration_bp: i32,
    pub duplication_bp: i32,
    /// Who ran the command (`--as`), informative.
    pub by: String,
    /// Where Nano approved these limits: a decision card or thread message
    /// id, a card comment, or a GitHub comment by `baileyrd`.
    pub source: String,
}

impl Record {
    fn check(&self) -> Result<(), String> {
        if self.concentration_bp < 0 || self.duplication_bp < 0 {
            return Err("limits are whole basis points, 0 or more".into());
        }
        if self.source.trim().is_empty() || self.source.contains(['\n', '\r']) {
            return Err("--source must name where Nano approved the limits, on one line".into());
        }
        if self.by.contains(['\n', '\r']) {
            return Err("--as must be one line".into());
        }
        Ok(())
    }
}

/// Parse the log. A malformed line, an invalid record, or a last line
/// without its `\n` (a torn append) is an error.
pub fn parse(text: &str) -> Result<Vec<Record>, String> {
    if !text.is_empty() && !text.ends_with('\n') {
        return Err("erosion limits: the last line has no newline (a torn append)".into());
    }
    text.lines()
        .enumerate()
        .map(|(i, line)| {
            let bad = |e: String| format!("erosion limits line {}: {e}", i + 1);
            let r: Record = rusty_serde::json::from_str(line).map_err(|e| bad(e.to_string()))?;
            r.check().map_err(bad)?;
            Ok(r)
        })
        .collect()
}

/// The newest approval, or `None` if Nano has approved none.
pub fn newest(dir: &Path) -> Result<Option<Record>, String> {
    let p = path(dir);
    let text = match std::fs::read_to_string(&p) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", p.display())),
    };
    Ok(parse(&text)?.pop())
}

/// Append an approval. The existing log must parse, so a damaged log is
/// repaired by hand rather than hidden behind a newer line.
pub fn approve(dir: &Path, record: Record) -> Result<Record, String> {
    record.check()?;
    let p = path(dir);
    let io = |e: std::io::Error| format!("{}: {e}", p.display());
    let mut file = open_locked(&p).map_err(io)?;
    let mut text = String::new();
    file.read_to_string(&mut text).map_err(io)?;
    parse(&text)?;
    let line = rusty_serde::json::to_string(&record).map_err(|e| e.to_string())?;
    file.write_all(format!("{line}\n").as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(io)?;
    Ok(record)
}

/// D115: an enforcing open's `(concentration, duplication)` limits must be
/// the newest record's.
pub fn check(dir: &Path, limits: (i32, i32)) -> Result<(), String> {
    let Some(r) = newest(dir)? else {
        return Err(
            "no approved limits are recorded; Nano records them with bbp erosion approve (D115)"
                .into(),
        );
    };
    match (r.concentration_bp, r.duplication_bp) == limits {
        true => Ok(()),
        false => Err(format!(
            "limits {} / {} bp differ from the approved {} / {} bp (D115)",
            limits.0, limits.1, r.concentration_bp, r.duplication_bp
        )),
    }
}

/// A record for `bbp erosion approve`, stamped `now`.
pub fn record(
    concentration_bp: i32,
    duplication_bp: i32,
    by: &str,
    source: &str,
    now: Time,
) -> Record {
    Record {
        at_ms: now.0,
        concentration_bp,
        duplication_bp,
        by: by.into(),
        source: source.into(),
    }
}

fn open_locked(p: &Path) -> std::io::Result<File> {
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = OpenOptions::new()
        .read(true)
        .append(true)
        .create(true)
        .open(p)?;
    file.lock()?;
    Ok(file)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    /// A fresh directory per call: tests run in parallel in one process, so
    /// the pid and clock alone can collide.
    fn dir() -> PathBuf {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!(
            "erosion-limits-{}-{}-{n}",
            std::process::id(),
            crate::now().0
        ));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn no_record_refuses_and_the_newest_record_must_match_exactly() {
        let d = dir();
        assert!(check(&d, (1, 1))
            .unwrap_err()
            .contains("no approved limits"));
        approve(&d, record(2, 2, "nano", "cmsg_a", Time(1))).unwrap();
        approve(&d, record(1, 1, "nano", "cmsg_b", Time(2))).unwrap();
        assert_eq!(newest(&d).unwrap().unwrap().source, "cmsg_b");
        check(&d, (1, 1)).unwrap();
        assert!(check(&d, (2, 2)).unwrap_err().contains("differ"));
        assert!(check(&d, (1, 0)).is_err());
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn a_record_needs_a_source_and_non_negative_limits() {
        let d = dir();
        assert!(approve(&d, record(1, 1, "nano", " ", Time(1))).is_err());
        assert!(approve(&d, record(1, 1, "nano", "a\nb", Time(1))).is_err());
        assert!(approve(&d, record(-1, 1, "nano", "cmsg", Time(1))).is_err());
        assert_eq!(newest(&d).unwrap(), None);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_damaged_log_fails_closed_and_blocks_new_approvals() {
        let d = dir();
        approve(&d, record(1, 1, "nano", "cmsg", Time(1))).unwrap();
        let p = path(&d);
        let mut text = std::fs::read_to_string(&p).unwrap();
        text.push_str("{\"at_ms\":2");
        std::fs::write(&p, &text).unwrap();
        assert!(check(&d, (1, 1)).unwrap_err().contains("torn"));
        std::fs::write(&p, "nonsense\n").unwrap();
        assert!(check(&d, (1, 1)).unwrap_err().contains("line 1"));
        assert!(approve(&d, record(1, 1, "nano", "cmsg", Time(3))).is_err());
        std::fs::remove_dir_all(d).unwrap();
    }
}
