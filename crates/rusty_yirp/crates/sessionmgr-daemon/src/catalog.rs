//! Reading and writing session records on disk.
//!
//! # The persistence split, and who owns what
//!
//! Two files per session, following `rusty_prime_agent`'s split directly:
//!
//! - `state.json` -- a small **pointer**: status, pids, fingerprints.
//!   Rewritten in full each time. This is all a restarting supervisor
//!   reads to rebuild its registry.
//! - `transcript.jsonl` -- the append-only **source of truth** for
//!   output. Never rewritten, only appended, so a crash mid-write costs
//!   at most a trailing partial line.
//!
//! **Ownership is split by lifecycle, not by file**, and this matters
//! because two processes can see the same `state.json`:
//!
//! | Moment | Writer |
//! |---|---|
//! | Session creation, before any worker exists | daemon |
//! | While the worker is alive | **the worker, exclusively** |
//! | After the daemon has established the worker is dead | daemon |
//! | Teardown, after the pids have been terminated | daemon |
//!
//! The daemon never writes a record belonging to a worker it believes is
//! alive, so the two never race in ordinary operation. The one residual
//! window is teardown -- which is why close terminates the processes
//! *first* and writes the record *after*, rather than the other way
//! round.

use std::path::Path;

use sessionmgr_core::ports::ProcessPort;
use sessionmgr_core::{
    decide_recovery, Liveness, RecoveryAction, Session, SessionId, SessionStatus,
};
use sessionmgr_proc::SystemProcessPort;
use sessionmgr_protocol::{SessionEvent, SessionSummary};

use crate::error::{Error, Result};
use crate::paths;

/// Writes a session record, atomically enough that a reader never sees a
/// half-written file.
///
/// Write-to-temp-then-rename: `rename` is atomic on both platforms
/// (Windows' `MoveFileEx` with `MOVEFILE_REPLACE_EXISTING`, which
/// `std::fs::rename` uses). Writing in place would leave a truncated
/// `state.json` if the process died mid-write -- and a supervisor that
/// cannot parse a session's record is exactly the situation this whole
/// module exists to survive.
pub fn write_session(root: &Path, session: &Session) -> Result<()> {
    let dir = paths::session_dir(root, &session.id);
    paths::ensure_dir("creating a session directory", &dir)?;
    let final_path = paths::session_state(root, &session.id);
    let temp_path = dir.join("state.json.tmp");
    let encoded = serde_json::to_string_pretty(session)?;
    std::fs::write(&temp_path, encoded)
        .map_err(|e| Error::io("writing a session record", temp_path.clone(), e))?;
    std::fs::rename(&temp_path, &final_path)
        .map_err(|e| Error::io("replacing a session record", final_path, e))
}

/// Reads one session record.
pub fn read_session(root: &Path, id: &SessionId) -> Result<Session> {
    let path = paths::session_state(root, id);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(Error::NotFound { id: id.to_string() })
        }
        Err(e) => return Err(Error::io("reading a session record", path, e)),
    };
    serde_json::from_str(&text).map_err(Error::from)
}

/// Every session on disk, oldest first.
///
/// A directory whose `state.json` is missing or unparseable is **skipped,
/// not fatal**: one corrupt record must not make `sessionmgr list` fail
/// for every other session. The id ordering comes free from
/// `SessionId`'s time-ordered prefix.
pub fn list_sessions(root: &Path) -> Result<Vec<Session>> {
    let dir = paths::sessions_dir(root);
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        // No sessions directory yet simply means no sessions.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(Error::io("listing sessions", dir, e)),
    };
    let mut sessions = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Ok(id) = name.parse::<SessionId>() else {
            continue;
        };
        if let Ok(session) = read_session(root, &id) {
            sessions.push(session);
        }
    }
    sessions.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(sessions)
}

/// Probes a session's recorded worker and returns the recovery decision.
///
/// The liveness question is asked in its PID-reuse-safe form -- pid *and*
/// start fingerprint -- because a bare pid check would report a dead
/// worker as alive whenever an unrelated process has since inherited its
/// number, and this supervisor would then decline to mark a session
/// crashed that genuinely is.
pub fn recovery_for(session: &Session) -> RecoveryAction {
    let port = SystemProcessPort;
    let liveness = session.worker.as_ref().map(|worker| {
        if port.is_same_process(worker.pid, worker.start_fingerprint.as_deref()) {
            Liveness::Alive
        } else {
            Liveness::Dead
        }
    });
    decide_recovery(session, liveness)
}

/// Applies [`recovery_for`] to a record, persisting a crash if one is
/// detected, and returns the possibly-updated record.
///
/// Used both on daemon startup (over every session) and on each read
/// during `list` -- a worker can die at any moment, not only while the
/// daemon happens to be restarting, so the same rule is applied on every
/// path that reports status rather than only at startup.
pub fn reconcile(root: &Path, mut session: Session) -> Result<Session> {
    match recovery_for(&session) {
        RecoveryAction::Adopt | RecoveryAction::LeaveAsIs => Ok(session),
        RecoveryAction::MarkCrashed => {
            // The transition can legitimately be refused if the record
            // moved on underneath us (a worker wrote `Finished` between
            // our read and now). Losing that race means the worker's own
            // account is the more recent one, so it wins.
            if session.transition_to(SessionStatus::Crashed).is_ok() {
                write_session(root, &session)?;
            }
            Ok(session)
        }
    }
}

pub fn summarize(session: &Session) -> SessionSummary {
    SessionSummary {
        id: session.id.clone(),
        kind: session.kind,
        status: session.status,
        command: session.command.clone(),
        cwd: session.workspace.as_ref().map(|w| w.cwd.clone()),
        branch: session.workspace.as_ref().and_then(|w| w.branch.clone()),
        created_at_millis: session.created_at_millis,
        exit_code: session.exit_code,
        agent: session.agent,
        name: session.name.clone(),
        parent: session.parent_id.clone(),
        forked_from: session.forked_from.clone(),
        switched_from: session.switched_from.clone(),
    }
}

/// Appends one event to a session's transcript.
///
/// Opened, written, and closed per call rather than holding the file
/// open. That costs a syscall per event and buys durability: a worker
/// killed uncleanly -- the case this project is built around -- leaves a
/// complete transcript up to its last event rather than losing whatever
/// sat in a buffer.
pub fn append_transcript(root: &Path, id: &SessionId, event: &SessionEvent) -> Result<()> {
    use std::io::Write;
    let path = paths::session_transcript(root, id);
    let mut line = serde_json::to_string(event)?;
    line.push('\n');
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| Error::io("opening a transcript", path.clone(), e))?;
    file.write_all(line.as_bytes())
        .map_err(|e| Error::io("appending to a transcript", path, e))
}

/// Reads a transcript back for replay to a newly attached client.
///
/// A trailing partial line -- the signature of a process killed
/// mid-append -- is **dropped rather than treated as corruption**. Losing
/// the last fragment of output from a crashed session is a much smaller
/// harm than refusing to show the user any of it.
pub fn read_transcript(root: &Path, id: &SessionId) -> Result<Vec<SessionEvent>> {
    let path = paths::session_transcript(root, id);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(Error::io("reading a transcript", path, e)),
    };
    Ok(text
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sessionmgr_core::SessionKind;

    struct TempRoot(std::path::PathBuf);

    impl TempRoot {
        fn new(label: &str) -> Self {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            (label, std::process::id(), nanos).hash(&mut hasher);
            let dir = std::env::temp_dir().join(format!("smgr{:x}", hasher.finish() & 0xffff_ffff));
            std::fs::create_dir_all(&dir).expect("create temp root");
            TempRoot(dir)
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn session() -> Session {
        Session::new(
            SessionId::new(1_700_000_000_000, 5),
            SessionKind::PlainTerminal,
            vec!["sh".to_owned()],
            None,
            true,
            1_700_000_000_000,
            None,
            None,
            false,
            None,
            None,
            None,
        )
    }

    #[test]
    fn a_written_record_reads_back_identically() {
        let root = TempRoot::new("roundtrip");
        let s = session();
        write_session(&root.0, &s).expect("write");
        assert_eq!(read_session(&root.0, &s.id).expect("read"), s);
    }

    #[test]
    fn writing_leaves_no_temp_file_behind() {
        let root = TempRoot::new("notemp");
        let s = session();
        write_session(&root.0, &s).expect("write");
        assert!(!paths::session_dir(&root.0, &s.id)
            .join("state.json.tmp")
            .exists());
    }

    #[test]
    fn reading_a_missing_session_is_not_found_not_an_io_error() {
        // The distinction the wire protocol depends on: `__hook-fire`
        // must be able to tell "not ours" from "something broke".
        let root = TempRoot::new("missing");
        let err = read_session(&root.0, &SessionId::new(1, 1)).expect_err("must fail");
        assert!(matches!(err, Error::NotFound { .. }));
    }

    #[test]
    fn listing_skips_corrupt_records_instead_of_failing_the_whole_list() {
        let root = TempRoot::new("corrupt");
        let good = session();
        write_session(&root.0, &good).expect("write");

        let bad_id = SessionId::new(1_700_000_000_001, 5);
        let bad_dir = paths::session_dir(&root.0, &bad_id);
        std::fs::create_dir_all(&bad_dir).expect("mkdir");
        std::fs::write(bad_dir.join("state.json"), "{ this is not json").expect("write junk");

        let listed = list_sessions(&root.0).expect("list");
        assert_eq!(listed.len(), 1, "one corrupt record must not hide the rest");
        assert_eq!(listed[0].id, good.id);
    }

    #[test]
    fn listing_an_empty_root_is_empty_not_an_error() {
        let root = TempRoot::new("empty");
        assert!(list_sessions(&root.0).expect("list").is_empty());
    }

    #[test]
    fn listing_returns_sessions_in_creation_order() {
        let root = TempRoot::new("order");
        let mut ids = Vec::new();
        for millis in [1_700_000_000_002u64, 1_700_000_000_000, 1_700_000_000_001] {
            let mut s = session();
            s.id = SessionId::new(millis, 1);
            ids.push(s.id.clone());
            write_session(&root.0, &s).expect("write");
        }
        let listed: Vec<_> = list_sessions(&root.0)
            .expect("list")
            .into_iter()
            .map(|s| s.id)
            .collect();
        let mut expected = ids;
        expected.sort();
        assert_eq!(listed, expected);
    }

    #[test]
    fn a_session_recorded_as_running_with_a_dead_worker_reconciles_to_crashed() {
        let root = TempRoot::new("reconcile");
        let mut s = session();
        s.status = SessionStatus::Running;
        s.worker = Some(sessionmgr_core::WorkerRef {
            // A pid that is certainly not a live process of ours, with a
            // fingerprint that cannot match even if it were.
            pid: 0xffff_fffe,
            start_fingerprint: Some("not-a-real-fingerprint".to_owned()),
        });
        write_session(&root.0, &s).expect("write");

        let reconciled = reconcile(&root.0, s.clone()).expect("reconcile");
        assert_eq!(reconciled.status, SessionStatus::Crashed);
        // And it was persisted, not just returned.
        assert_eq!(
            read_session(&root.0, &s.id).expect("read").status,
            SessionStatus::Crashed
        );
    }

    #[test]
    fn a_live_worker_is_adopted_and_the_record_is_left_alone() {
        let root = TempRoot::new("adopt");
        let mut s = session();
        s.status = SessionStatus::Running;
        // This very test process: certainly alive, and its fingerprint
        // certainly matches itself.
        let me = std::process::id();
        s.worker = Some(sessionmgr_core::WorkerRef {
            pid: me,
            start_fingerprint: sessionmgr_proc::start_fingerprint(me).ok().flatten(),
        });
        write_session(&root.0, &s).expect("write");

        assert_eq!(
            reconcile(&root.0, s.clone()).expect("reconcile").status,
            SessionStatus::Running,
            "a live worker must be adopted, never marked crashed"
        );
    }

    #[test]
    fn transcripts_append_and_replay_in_order() {
        let root = TempRoot::new("transcript");
        let s = session();
        write_session(&root.0, &s).expect("write");
        for text in ["first", "second", "third"] {
            append_transcript(
                &root.0,
                &s.id,
                &SessionEvent::Output {
                    data: text.as_bytes().to_vec(),
                },
            )
            .expect("append");
        }
        let replayed = read_transcript(&root.0, &s.id).expect("read");
        assert_eq!(replayed.len(), 3);
        assert_eq!(
            replayed[0],
            SessionEvent::Output {
                data: b"first".to_vec()
            }
        );
    }

    #[test]
    fn a_truncated_trailing_line_is_dropped_not_treated_as_corruption() {
        // The signature of a worker killed mid-append -- the normal case
        // for a tool built around surviving unclean exits. The earlier
        // complete events must still be readable.
        let root = TempRoot::new("truncated");
        let s = session();
        write_session(&root.0, &s).expect("write");
        append_transcript(
            &root.0,
            &s.id,
            &SessionEvent::Output {
                data: b"complete".to_vec(),
            },
        )
        .expect("append");
        {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(paths::session_transcript(&root.0, &s.id))
                .expect("open");
            f.write_all(b"{\"type\":\"output\",\"data\":\"dHJ1bmM")
                .expect("write partial");
        }
        let replayed = read_transcript(&root.0, &s.id).expect("read");
        assert_eq!(replayed.len(), 1);
    }

    #[test]
    fn reading_a_transcript_that_does_not_exist_yet_is_empty() {
        let root = TempRoot::new("notranscript");
        assert!(read_transcript(&root.0, &SessionId::new(1, 1))
            .expect("read")
            .is_empty());
    }
}
