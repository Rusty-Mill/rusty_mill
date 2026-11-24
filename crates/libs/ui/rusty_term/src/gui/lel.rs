//! LEL tab view model (card #1416, slice 1): the Mill Board decision inbox as
//! display rows.
//!
//! Pure parsing, no UI and no I/O. The input is the JSON `inbox_list` returns
//! (an array of escalations, each with a `label` and its board `card`). The
//! types stay here rather than in `rusty_mill_board` (D174); a drift test in
//! that crate parses `testdata/lel_inbox.json` with its own types.

use rusty_json::Value;

/// A decision-inbox row, one open escalation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Row {
    /// `<task>/<subject>/<seq>`; the id an action names.
    pub id: String,
    /// The task revision this was read at; every action is fenced on it.
    pub rev: u64,
    /// Why the human is asked, in plain words (the host's `label`).
    pub reason: String,
    /// A whole-task stop: no claim, the task waits.
    pub task_level: bool,
    /// The escalated claim; `None` for a task total.
    pub claim: Option<Claim>,
    /// A stuck question's live candidates, side by side.
    pub candidates: Vec<Claim>,
    /// `Some(false)` for an irreversible claim.
    pub reversible: Option<bool>,
    /// `Low`, `Medium` or `High`.
    pub stakes: Option<String>,
    /// The board card that owns the task.
    pub card: Option<Card>,
}

/// A claim, reduced to what the tab shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Claim {
    /// The claim message id.
    pub id: u64,
    /// The claim text.
    pub text: String,
    /// BBP status name.
    pub status: String,
    /// Confidence as a whole percent.
    pub confidence: u32,
    /// Budget units spent.
    pub spent: u64,
    /// Budget units available.
    pub budget: u64,
}

/// The board card an escalation belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Card {
    /// Card number, once assigned.
    pub number: Option<u64>,
    /// Card title.
    pub title: String,
}

/// Why an inbox read could not be shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Error {
    /// The text is not JSON.
    Json(String),
    /// The JSON is not an array of escalations.
    NotAnArray,
    /// Entry `index` lacks a field, or it has the wrong type.
    Field {
        /// Position in the array.
        index: usize,
        /// The field name.
        name: &'static str,
    },
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Json(e) => write!(f, "the inbox reply is not JSON: {e}"),
            Self::NotAnArray => f.write_str("the inbox reply is not a list"),
            Self::Field { index, name } => {
                write!(f, "inbox entry {index} has no usable `{name}`")
            }
        }
    }
}

/// Reasons that stop a whole task, as BBP names them.
const TASK_REASONS: [&str; 2] = ["TaskTotal", "TaskPaused"];

/// Parses an `inbox_list` reply. An empty list is an empty inbox, not an error.
///
/// # Errors
/// [`Error`] on text that is not JSON, not a list, or an entry missing `id`,
/// `rev` or `reason`. One bad entry refuses the read: a half-shown inbox could
/// hide a decision.
pub(crate) fn parse(text: &str) -> Result<Vec<Row>, Error> {
    let value = Value::parse(text).map_err(|e| Error::Json(e.to_string()))?;
    let items = value.as_array().ok_or(Error::NotAnArray)?;
    items.iter().enumerate().map(|(i, v)| row(i, v)).collect()
}

fn row(index: usize, v: &Value) -> Result<Row, Error> {
    let miss = |name| Error::Field { index, name };
    let id = v.get("id").and_then(Value::as_str).ok_or(miss("id"))?;
    let rev = v.get("rev").and_then(Value::as_u64).ok_or(miss("rev"))?;
    let kind = v
        .get("reason")
        .and_then(Value::as_str)
        .ok_or(miss("reason"))?;
    let text = |k| v.get(k).and_then(Value::as_str).map(str::to_owned);
    Ok(Row {
        id: id.to_owned(),
        rev,
        // An unknown reason still shows, under the host's label or its name.
        reason: text("label").unwrap_or_else(|| kind.to_owned()),
        task_level: TASK_REASONS.contains(&kind),
        claim: v.get("claim").and_then(claim),
        candidates: v
            .get("candidates")
            .and_then(Value::as_array)
            .map_or_else(Vec::new, |a| a.iter().filter_map(claim).collect()),
        reversible: v.get("reversible").and_then(Value::as_bool),
        stakes: text("stakes"),
        card: v.get("card").and_then(card),
    })
}

fn claim(v: &Value) -> Option<Claim> {
    let n = |k| v.get(k).and_then(Value::as_u64);
    Some(Claim {
        id: n("id")?,
        text: v.get("text")?.as_str()?.to_owned(),
        status: v.get("status")?.as_str()?.to_owned(),
        confidence: u32::try_from(n("confidence")?).ok()?,
        spent: n("spent")?,
        budget: n("budget")?,
    })
}

fn card(v: &Value) -> Option<Card> {
    Some(Card {
        number: v.get("number").and_then(Value::as_u64),
        title: v.get("title")?.as_str()?.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("testdata/lel_inbox.json");

    #[test]
    fn reads_the_golden_inbox() {
        let rows = parse(FIXTURE).unwrap();
        assert_eq!(rows.len(), 3);
        let a = &rows[0];
        assert_eq!((a.id.as_str(), a.rev), ("T1/msg:12/3", 41));
        assert_eq!(a.reason, "Irreversible action");
        assert_eq!(a.reversible, Some(false));
        assert_eq!(a.stakes.as_deref(), Some("High"));
        assert!(!a.task_level);
        assert_eq!(
            a.claim.as_ref().map(|c| (c.id, c.spent, c.budget)),
            Some((12, 30, 100))
        );
        assert_eq!(a.card.as_ref().and_then(|c| c.number), Some(1416));
    }

    #[test]
    fn a_stuck_question_lists_its_candidates() {
        let rows = parse(FIXTURE).unwrap();
        let ids: Vec<u64> = rows[1].candidates.iter().map(|c| c.id).collect();
        assert_eq!(ids, [21, 22]);
        assert!(rows[1].card.is_none());
    }

    #[test]
    fn a_task_total_has_no_claim() {
        let rows = parse(FIXTURE).unwrap();
        assert!(rows[2].task_level);
        assert!(rows[2].claim.is_none());
    }

    #[test]
    fn an_empty_inbox_is_empty() {
        assert_eq!(parse("[]"), Ok(vec![]));
    }

    #[test]
    fn unknown_reason_shows_its_name() {
        let rows = parse(r#"[{"id":"T/x/1","rev":1,"reason":"Novel"}]"#).unwrap();
        assert_eq!(rows[0].reason, "Novel");
        assert!(!rows[0].task_level);
    }

    #[test]
    fn errors_are_named() {
        assert!(matches!(parse("not json"), Err(Error::Json(_))));
        assert_eq!(parse("{}"), Err(Error::NotAnArray));
        assert_eq!(
            parse(r#"[{"id":"a","reason":"Stuck"}]"#),
            Err(Error::Field {
                index: 0,
                name: "rev"
            })
        );
        let two = r#"[{"id":"a","rev":1,"reason":"Stuck"},{"rev":2,"reason":"Stuck"}]"#;
        assert_eq!(
            parse(two),
            Err(Error::Field {
                index: 1,
                name: "id"
            })
        );
    }

    #[test]
    fn a_malformed_claim_is_dropped_not_guessed() {
        let rows = parse(r#"[{"id":"a","rev":1,"reason":"Stuck","claim":{"id":1}}]"#).unwrap();
        assert!(rows[0].claim.is_none());
    }
}
