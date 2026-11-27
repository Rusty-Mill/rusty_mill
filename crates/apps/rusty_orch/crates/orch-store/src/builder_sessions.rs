//! The on-disk shape of one BBP board's builder sessions (#600, card
//! #1404): the `builder_session` table, one record per board holding every
//! session row, so admission reads the whole board in one get.
//!
//! Positional bincode under the schema tag below: the v1 layout is the
//! format, and any layout change needs a new tag (see `record.rs`).

use orch_lens::builder::{self, End, Phase, ProfileResult, RunResult, Session};
use rusty_bbp::{ArtId, MsgId};
use rusty_multimodal_db_engine::generic::traits::{
    IndexedField, Record, ScannableField, SchemaTag,
};
use serde::{Deserialize, Serialize};

use crate::record::{ById, Revision};
use crate::StoreError;

/// Engine key for a board, as for lens states, with its own prefix.
pub fn key(board: &str) -> i64 {
    crate::lens_states::key(&format!("builders:{board}"))
}

/// The state words of the table.
const ADMITTED: &str = "admitted";
const RUNNING: &str = "running";
const STORED: &str = "stored";
const FAILED: &str = "failed";
const ABORTED: &str = "aborted";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RunRow {
    /// `(profile, passed)`.
    profiles: Vec<(String, bool)>,
    erosion_delta: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SessionRow {
    claim: u64,
    n: u32,
    candidate: Option<u64>,
    state: String,
    started: u64,
    ended: Option<u64>,
    tokens_in: u64,
    tokens_out: u64,
    run: Option<RunRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuilderSessionRecord {
    id: i64,
    pub revision: i64,
    pub board: String,
    rows: Vec<SessionRow>,
}

impl BuilderSessionRecord {
    pub fn new(board: &str, sessions: &[Session], revision: u64) -> Self {
        Self {
            id: key(board),
            revision: revision as i64,
            board: board.to_owned(),
            rows: sessions.iter().map(row).collect(),
        }
    }

    /// The sessions, once the board and every row check out.
    pub fn into_sessions(self, board: &str) -> Result<Vec<Session>, StoreError> {
        if self.board != board {
            return Err(StoreError::Corrupt(format!(
                "builder session key for {board:?} holds {:?}",
                self.board
            )));
        }
        let sessions = self
            .rows
            .into_iter()
            .map(session)
            .collect::<Result<Vec<_>, _>>()?;
        builder::check(&sessions).map_err(StoreError::Corrupt)?;
        Ok(sessions)
    }
}

fn row(s: &Session) -> SessionRow {
    let (state, ended) = match s.phase {
        Phase::Admitted => (ADMITTED, None),
        Phase::Running => (RUNNING, None),
        Phase::Ended { end, at } => (end_word(end), Some(at)),
    };
    SessionRow {
        claim: s.claim.0,
        n: s.n,
        candidate: s.candidate().map(|a| a.0),
        state: state.to_owned(),
        started: s.started,
        ended,
        tokens_in: s.tokens_in,
        tokens_out: s.tokens_out,
        run: s.run.as_ref().map(|r| RunRow {
            profiles: r
                .profiles
                .iter()
                .map(|p| (p.profile.clone(), p.passed))
                .collect(),
            erosion_delta: r.erosion_delta,
        }),
    }
}

fn end_word(end: End) -> &'static str {
    match end {
        End::Stored(_) => STORED,
        End::Failed => FAILED,
        End::Aborted => ABORTED,
    }
}

fn session(r: SessionRow) -> Result<Session, StoreError> {
    let phase = phase(&r)?;
    Ok(Session {
        claim: MsgId(r.claim),
        n: r.n,
        started: r.started,
        phase,
        tokens_in: r.tokens_in,
        tokens_out: r.tokens_out,
        run: r.run.map(|run| RunResult {
            profiles: run
                .profiles
                .into_iter()
                .map(|(profile, passed)| ProfileResult { profile, passed })
                .collect(),
            erosion_delta: run.erosion_delta,
        }),
    })
}

/// The row's phase: a candidate only on `stored`, an end time only once
/// ended.
fn phase(r: &SessionRow) -> Result<Phase, StoreError> {
    let bad = || {
        StoreError::Corrupt(format!(
            "builder session msg:{} #{}: state {:?} with candidate {:?} and end {:?}",
            r.claim, r.n, r.state, r.candidate, r.ended
        ))
    };
    let ended = |end: End| r.ended.map(|at| Phase::Ended { end, at }).ok_or_else(bad);
    match (r.state.as_str(), r.candidate, r.ended) {
        (ADMITTED, None, None) => Ok(Phase::Admitted),
        (RUNNING, None, None) => Ok(Phase::Running),
        (STORED, Some(a), _) => ended(End::Stored(ArtId(a))),
        (FAILED, None, _) => ended(End::Failed),
        (ABORTED, None, _) => ended(End::Aborted),
        _ => Err(bad()),
    }
}

impl Record for BuilderSessionRecord {
    type Id = i64;
    fn id(&self) -> i64 {
        self.id
    }
}

impl SchemaTag for BuilderSessionRecord {
    const SCHEMA_TAG: &'static str = "rusty_orch::BuilderSessionRecord::v1";
}

impl IndexedField<ById> for BuilderSessionRecord {
    type IndexValue = i64;
    fn indexed_value(&self) -> &i64 {
        &self.id
    }
}

impl ScannableField<Revision> for BuilderSessionRecord {
    type ScanValue = i64;
    fn scannable_value(&self) -> i64 {
        self.revision
    }
    fn set_scannable_value(&mut self, value: i64) {
        self.revision = value;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(state: &str, candidate: Option<u64>, ended: Option<u64>) -> SessionRow {
        SessionRow {
            claim: 7,
            n: 1,
            candidate,
            state: state.to_owned(),
            started: 1,
            ended,
            tokens_in: 0,
            tokens_out: 0,
            run: None,
        }
    }

    #[test]
    fn phases_need_their_columns() {
        assert!(phase(&raw(ADMITTED, None, None)).is_ok());
        assert!(phase(&raw(RUNNING, None, None)).is_ok());
        assert!(phase(&raw(STORED, Some(3), Some(5))).is_ok());
        assert!(phase(&raw(FAILED, None, Some(5))).is_ok());
        assert!(phase(&raw(ABORTED, None, Some(5))).is_ok());
        for bad in [
            raw(ADMITTED, None, Some(5)),
            raw(RUNNING, Some(3), None),
            raw(STORED, None, Some(5)),
            raw(STORED, Some(3), None),
            raw(FAILED, Some(3), Some(5)),
            raw(ABORTED, None, None),
            raw("paused", None, None),
        ] {
            assert!(phase(&bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_record_for_another_board_is_refused() {
        let r = BuilderSessionRecord::new("a", &[], 1);
        assert!(matches!(r.into_sessions("b"), Err(StoreError::Corrupt(_))));
    }
}
