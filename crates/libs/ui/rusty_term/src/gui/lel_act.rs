//! LEL tab decisions (card #1416, slice 4): Approve, Reject and Choose.
//!
//! Pure view model, no UI and no I/O. It builds the `inbox_act` arguments
//! (the displayed `rev` is the fence, a `dry_run` always goes first, D173),
//! the D58 confirm, and reads the answer. A stale `rev` is refused and
//! re-read, never retried: the escalation may have moved, so the human looks
//! again. Nothing here launches a build: "Try both builds" only lays the
//! candidates side by side (Q43 interim, D222).

use super::lel::{Claim, Row};

/// What the human decides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Act {
    /// Accept the claim as shown.
    Approve,
    /// Drop it. A whole-task stop needs a reason.
    Reject(Option<String>),
    /// Taste call: keep this candidate (claim id), drop the others.
    Choose(u64),
}

/// Why an action cannot be sent at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// Rejecting a whole-task stop needs a non-blank reason.
    NeedsReason,
    /// Choose names a claim that is not one of the row's candidates.
    NotACandidate(u64),
    /// Choose on a row with no candidates.
    NothingToChoose,
    /// Approve on a taste call: it must name a candidate.
    ChooseInstead,
}

/// One `inbox_act` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Request {
    id: String,
    rev: u64,
    act: Act,
    dry_run: bool,
}

impl Request {
    /// The preview of `act` on `row`, at the revision the human is looking at.
    ///
    /// # Errors
    /// [`Refusal`] when the action does not fit the row; nothing is sent.
    pub(crate) fn preview(row: &Row, act: Act) -> Result<Self, Refusal> {
        let act = match act {
            Act::Reject(r) if row.task_level => Act::Reject(Some(
                r.filter(|s| !s.trim().is_empty())
                    .ok_or(Refusal::NeedsReason)?,
            )),
            Act::Reject(_) => Act::Reject(None),
            Act::Choose(_) if row.candidates.is_empty() => return Err(Refusal::NothingToChoose),
            Act::Choose(id) if !row.candidates.iter().any(|c| c.id == id) => {
                return Err(Refusal::NotACandidate(id));
            }
            Act::Approve if !row.candidates.is_empty() => return Err(Refusal::ChooseInstead),
            a => a,
        };
        Ok(Self {
            id: row.id.clone(),
            rev: row.rev,
            act,
            dry_run: true,
        })
    }

    /// The same decision for real, once the human has seen the preview. Same id
    /// and the same `rev`: a changed escalation is fenced, never re-fenced here.
    pub(crate) fn apply(&self) -> Self {
        Self {
            dry_run: false,
            ..self.clone()
        }
    }

    /// Whether this call writes nothing.
    pub(crate) fn is_dry_run(&self) -> bool {
        self.dry_run
    }

    /// The `inbox_act` arguments as JSON text.
    pub(crate) fn args(&self) -> String {
        let head = format!(
            "{{\"id\":{},\"rev\":{},\"dry_run\":{}",
            quote(&self.id),
            self.rev,
            self.dry_run
        );
        let tail = match &self.act {
            Act::Approve => ",\"action\":\"approve\"".to_owned(),
            Act::Reject(None) => ",\"action\":\"reject\"".to_owned(),
            Act::Reject(Some(r)) => format!(",\"action\":\"reject\",\"reason\":{}", quote(r)),
            Act::Choose(c) => format!(",\"action\":\"choose\",\"chosen\":{c}"),
        };
        format!("{head}{tail}}}")
    }
}

/// A JSON string literal.
fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// What the host said to an `inbox_act`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Answer {
    /// A dry run: the host's one-line plan is ready to show ("Dry run attached").
    Previewed,
    /// The decision was recorded.
    Done,
    /// The escalation moved on: re-read the inbox, show it again, do not resend.
    Stale(String),
    /// Refused for another reason; shown in the host's words.
    Refused(String),
}

/// Reads the host's reply. `ok` is whether the call succeeded; an error body is
/// the host's text (`stale_rev: ...` for a moved escalation).
pub(crate) fn answer(req: &Request, ok: bool, body: &str) -> Answer {
    match (ok, req.dry_run) {
        (true, true) => Answer::Previewed,
        (true, false) => Answer::Done,
        (false, _) if body.starts_with("stale_rev") => Answer::Stale(body.to_owned()),
        (false, _) => Answer::Refused(body.to_owned()),
    }
}

/// A confirm button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Button {
    /// Only dismisses.
    Cancel,
    /// Drops the claim.
    Reject,
    /// Primary: accepts the claim as shown.
    Accept,
}

/// The D58 confirm for an irreversible claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Confirm {
    /// "Approve the {claim} claim?"
    pub title: String,
    /// What approving does and does not do; never mentions running anything.
    pub body: &'static str,
    /// Left to right; the first one has focus.
    pub buttons: [Button; 3],
}

impl Confirm {
    /// The confirm for Approve on `row`; `None` when the claim can be undone
    /// (those approve directly).
    pub(crate) fn approve(row: &Row) -> Option<Self> {
        if row.reversible != Some(false) {
            return None;
        }
        let claim = row.claim.as_ref().map_or("this", |c| c.text.as_str());
        Some(Self {
            title: format!("Approve the {claim} claim?"),
            body: "Approving accepts the claim as shown. It adds no budget and does not run the action.",
            buttons: [Button::Cancel, Button::Reject, Button::Accept],
        })
    }

    /// The button that has focus first: always Cancel (D58).
    pub(crate) fn focus(&self) -> Button {
        self.buttons[0]
    }
}

/// "Try both builds": the candidates side by side. Pure display; launching
/// anything is not part of v0 (Q43).
pub(crate) fn side_by_side(row: &Row) -> Vec<&Claim> {
    row.candidates.iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::lel::parse;

    const FIXTURE: &str = include_str!("testdata/lel_inbox.json");

    fn rows() -> Vec<Row> {
        parse(FIXTURE).unwrap()
    }

    #[test]
    fn preview_comes_first_and_carries_the_displayed_rev() {
        let r = &rows()[0];
        let p = Request::preview(r, Act::Approve).unwrap();
        assert!(p.is_dry_run());
        let v = rusty_json::Value::parse(&p.args()).unwrap();
        assert_eq!(v.get("id").and_then(|x| x.as_str()), Some("T1/msg:12/3"));
        assert_eq!(v.get("rev").and_then(|x| x.as_u64()), Some(41));
        assert_eq!(v.get("action").and_then(|x| x.as_str()), Some("approve"));
        assert_eq!(v.get("dry_run").and_then(|x| x.as_bool()), Some(true));
        let a = p.apply();
        assert!(!a.is_dry_run());
        assert!(a.args().contains("\"rev\":41") && a.args().contains("\"dry_run\":false"));
    }

    #[test]
    fn choose_must_name_a_candidate() {
        let rows = rows();
        let ok = Request::preview(&rows[1], Act::Choose(21)).unwrap();
        assert!(ok.args().contains("\"chosen\":21"));
        assert_eq!(
            Request::preview(&rows[1], Act::Choose(99)),
            Err(Refusal::NotACandidate(99))
        );
        assert_eq!(
            Request::preview(&rows[0], Act::Choose(21)),
            Err(Refusal::NothingToChoose)
        );
        assert_eq!(
            Request::preview(&rows[1], Act::Approve),
            Err(Refusal::ChooseInstead)
        );
    }

    #[test]
    fn a_task_stop_needs_a_reason_to_reject() {
        let rows = rows();
        assert_eq!(
            Request::preview(&rows[2], Act::Reject(None)),
            Err(Refusal::NeedsReason)
        );
        assert_eq!(
            Request::preview(&rows[2], Act::Reject(Some("  ".into()))),
            Err(Refusal::NeedsReason)
        );
        let p = Request::preview(&rows[2], Act::Reject(Some("out of scope \"x\"".into()))).unwrap();
        let v = rusty_json::Value::parse(&p.args()).unwrap();
        assert_eq!(
            v.get("reason").and_then(|x| x.as_str()),
            Some("out of scope \"x\"")
        );
        // A claim reject sends no reason.
        let c = Request::preview(&rows[0], Act::Reject(Some("x".into()))).unwrap();
        assert!(!c.args().contains("reason"));
    }

    #[test]
    fn a_stale_rev_is_shown_not_retried() {
        let r = &rows()[0];
        let p = Request::preview(r, Act::Approve).unwrap();
        assert_eq!(answer(&p, true, "{}"), Answer::Previewed);
        assert_eq!(answer(&p.apply(), true, "{}"), Answer::Done);
        let msg = "stale_rev: T1/msg:12/3 is at rev 42, not 41; re-read the inbox";
        assert_eq!(answer(&p.apply(), false, msg), Answer::Stale(msg.into()));
        assert_eq!(
            answer(&p, false, "closed"),
            Answer::Refused("closed".into())
        );
    }

    #[test]
    fn irreversible_approve_confirms_with_cancel_first() {
        let rows = rows();
        let c = Confirm::approve(&rows[0]).unwrap();
        assert_eq!(c.title, "Approve the Drop the legacy export table claim?");
        assert_eq!(c.focus(), Button::Cancel);
        assert_eq!(c.buttons, [Button::Cancel, Button::Reject, Button::Accept]);
        let words = c.body.to_lowercase();
        assert!(words.contains("does not run the action"));
        assert!(!words.contains("move") && !words.contains("execut"));
        assert!(Confirm::approve(&rows[1]).is_none());
    }

    #[test]
    fn both_builds_are_just_the_candidates() {
        let rows = rows();
        let ids: Vec<u64> = side_by_side(&rows[1]).iter().map(|c| c.id).collect();
        assert_eq!(ids, [21, 22]);
        assert!(side_by_side(&rows[0]).is_empty());
    }
}
