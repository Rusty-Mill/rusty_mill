//! The repos whose tasks may open with scratch gates (#447, D195).
//!
//! Only `bbp gates scratch-add` writes `<dir>/gates/scratch-repos.log`: one JSON line per repo,
//! appended under the file's lock. Each line names where Nano approved it (`source`); like the
//! erosion limits (D115) that is an attestation the host records, not one it verifies. A task
//! opened `--gates scratch` on a repo not listed is refused before anything is written, so its
//! human gates stay owned: a plan or merge approval then needs a board receipt.

use rusty_bbp::Time;
use rusty_serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// `<dir>/gates/scratch-repos.log`.
pub fn path(dir: &Path) -> PathBuf {
    dir.join("gates").join("scratch-repos.log")
}

/// One listed repo.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub at_ms: u64,
    /// The repo exactly as `bbp open --repo` names it.
    pub repo: String,
    /// Who ran the command (`--as`), informative.
    pub by: String,
    /// Where Nano approved listing it: a decision card or thread message id, a card comment, or
    /// a GitHub comment by `baileyrd`.
    pub source: String,
}

impl Record {
    /// A record for `bbp gates scratch-add`, stamped `now`.
    pub fn new(repo: &str, by: &str, source: &str, now: Time) -> Record {
        Record {
            at_ms: now.0,
            repo: repo.into(),
            by: by.into(),
            source: source.into(),
        }
    }

    fn check(&self) -> Result<(), String> {
        let one_line = |s: &str| !s.trim().is_empty() && !s.contains(['\n', '\r']);
        if !one_line(&self.repo) {
            return Err("--repo must name the repo, on one line".into());
        }
        if !one_line(&self.source) {
            return Err("--source must name where Nano approved the repo, on one line".into());
        }
        if self.by.contains(['\n', '\r']) {
            return Err("--as must be one line".into());
        }
        Ok(())
    }
}

/// Parse the log. A malformed line, an invalid record, or a torn last line is an error.
pub fn parse(text: &str) -> Result<Vec<Record>, String> {
    if !text.is_empty() && !text.ends_with('\n') {
        return Err("scratch repos: the last line has no newline (a torn append)".into());
    }
    text.lines()
        .enumerate()
        .map(|(i, line)| {
            let bad = |e: String| format!("scratch repos line {}: {e}", i + 1);
            let r: Record = rusty_serde::json::from_str(line).map_err(|e| bad(e.to_string()))?;
            r.check().map_err(bad)?;
            Ok(r)
        })
        .collect()
}

/// Every listed repo, oldest first; none when the log does not exist.
pub fn list(dir: &Path) -> Result<Vec<Record>, String> {
    let p = path(dir);
    match std::fs::read_to_string(&p) {
        Ok(text) => parse(&text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(format!("{}: {e}", p.display())),
    }
}

/// List a repo. The existing log must parse, so damage is repaired by hand, not hidden.
pub fn add(dir: &Path, record: Record) -> Result<Record, String> {
    record.check()?;
    let p = path(dir);
    let io = |e: std::io::Error| format!("{}: {e}", p.display());
    let mut file = crate::erosion_limits::open_locked(&p).map_err(io)?;
    let mut text = String::new();
    file.read_to_string(&mut text).map_err(io)?;
    parse(&text)?;
    let line = rusty_serde::json::to_string(&record).map_err(|e| e.to_string())?;
    file.write_all(format!("{line}\n").as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(io)?;
    Ok(record)
}

/// Refuse a scratch open on a repo not listed. A damaged log refuses too: fail closed.
pub fn check(dir: &Path, repo: &str) -> Result<(), String> {
    match list(dir)?.iter().any(|r| r.repo == repo) {
        true => Ok(()),
        false => Err(format!(
            "{repo} is not a scratch repo; Nano lists one with bbp gates scratch-add (D195), \
             otherwise the task opens owned"
        )),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn dir() -> PathBuf {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("scratch-repos-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn only_a_listed_repo_passes_and_each_entry_names_its_source() {
        let d = dir();
        assert!(check(&d, "r").unwrap_err().contains("not a scratch repo"));
        add(&d, Record::new("r", "nano", "cmsg_a", Time(1))).unwrap();
        check(&d, "r").unwrap();
        assert!(check(&d, "other").is_err());
        assert_eq!(list(&d).unwrap()[0].source, "cmsg_a");
    }

    #[test]
    fn a_record_without_a_source_or_repo_is_refused() {
        let d = dir();
        assert!(add(&d, Record::new("r", "nano", " ", Time(1))).is_err());
        assert!(add(&d, Record::new("", "nano", "cmsg", Time(1))).is_err());
        assert!(list(&d).unwrap().is_empty());
    }

    #[test]
    fn a_damaged_log_refuses_every_scratch_open() {
        let d = dir();
        add(&d, Record::new("r", "nano", "cmsg", Time(1))).unwrap();
        let p = path(&d);
        std::fs::write(&p, std::fs::read_to_string(&p).unwrap().trim_end()).unwrap();
        assert!(check(&d, "r").unwrap_err().contains("torn"));
    }
}
