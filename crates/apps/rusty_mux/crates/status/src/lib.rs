//! Program status for rusty_mux blocks: what the program running in a block
//! says it is doing, reported with OSC 7501, the public Program Status
//! Protocol (draft 0.4).
//!
//! [`parse`] turns a report's body into a [`Report`], or nothing when the
//! protocol says to ignore or discard it. [`Records`] applies reports and the
//! terminal events that end records, and keeps at most [`MAX_RECORDS`]. No
//! I/O; the server feeds it what the block's grid collected.
//!
//! ```
//! use rusty_mux_status::{Records, State, parse};
//!
//! let mut records = Records::default();
//! records.apply(parse("state=working:app=cargo:progress=40").expect("valid"));
//! let (_, record) = records.iter().next().expect("one record");
//! assert_eq!((record.state, record.progress), (State::Working, Some(40)));
//! records.prompt(); // a new shell prompt ends a working record
//! assert!(records.iter().next().is_none());
//! ```

mod parse;
mod records;

pub use parse::parse;
pub use records::{MAX_RECORDS, Records};

/// What a program says it is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum State {
    /// At rest, waiting for its next instruction.
    Idle,
    /// Running; ends at the next prompt.
    Working,
    /// Waiting on the user; ends at the next prompt.
    Blocked,
    /// Finished with a result the user hasn't seen.
    Done,
    /// Failed and stopped.
    Error,
}

/// Why a [`State::Blocked`] program waits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    /// Approve an action.
    Permission,
    /// Answer a question.
    Question,
    /// Sign in or give a credential.
    Auth,
}

/// A record path: `/`-separated segments, or empty for the root record.
/// Always well formed; [`parse`] refuses a report with a bad one.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Id(String);

impl Id {
    /// The path as reported; empty for the root.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    /// Whether `self` is `other` or lies beneath it.
    fn within(&self, other: &Id) -> bool {
        other.is_root()
            || self.0 == other.0
            || self
                .0
                .strip_prefix(&other.0)
                .is_some_and(|rest| rest.starts_with('/'))
    }

    /// This path's ancestors, nearest first, ending with the root.
    fn ancestors(&self) -> impl Iterator<Item = Id> + '_ {
        let cuts = self
            .0
            .rmatch_indices('/')
            .map(|(i, _)| Id(self.0[..i].to_owned()));
        cuts.chain((!self.is_root()).then(Id::default))
    }
}

/// One program's report, as the last report for its [`Id`] left it. Text is
/// decoded and holds no control characters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub state: State,
    /// Only with [`State::Blocked`].
    pub kind: Option<Kind>,
    /// 0 to 100, only while working or blocked; `None` is indeterminate.
    pub progress: Option<u8>,
    /// A stable program identifier, not a label.
    pub app: Option<String>,
    pub title: Option<String>,
    pub msg: Option<String>,
}

/// A report that [`parse`] accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Report {
    /// Replace the record at the id.
    Set(Id, Record),
    /// Remove the record at the id and every record beneath it; the root
    /// clears everything.
    Clear(Id),
}
