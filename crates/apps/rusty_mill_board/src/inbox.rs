//! The decision inbox (#595): open BBP escalations and the human actions on them.
//!
//! Pure domain. The truth is the BBP log on the orch host; Mill Board reads it
//! through the [`Escalations`] port and stores nothing of its own. Each action
//! maps to one or more `bbp human` commands ([`plan`]), every one of them
//! fenced on the revision the escalation was read at, so an action against
//! newer work is refused by BBP rather than applied.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Longest Refine note, in characters (BBP's R8 message limit).
pub const NOTE_MAX: usize = 1_200;

/// Why the human is asked, in BBP's words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reason {
    /// The claim's action cannot be undone.
    Irreversible,
    /// The claim would replace one that was already acted on.
    SupersedesActed,
    /// The claim spent its evidence budget.
    Budget,
    /// No convergence: a deadlock, or a taste call between candidates.
    Stuck,
    /// A claim this one rests on was dropped.
    DepCollapsed,
    /// Work already shipped was contradicted.
    ReopenedAfterAction,
    /// An action mark never reported back.
    ActionUnconfirmed,
    /// A rerun is waiting on budget.
    RerunHeld,
    /// The whole task crossed a budget total and paused.
    TaskTotal,
    /// The intent changed under work already shipped (#598).
    IntentChanged,
    /// The host paused the whole task on a ceiling or the cost breaker (#598).
    TaskPaused,
}

impl Reason {
    /// A whole-task stop: the task waits, not one claim.
    #[must_use]
    pub const fn is_task(self) -> bool {
        matches!(self, Self::TaskTotal | Self::TaskPaused)
    }
}

impl Reason {
    /// The reason in plain words, as the inbox shows it.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Irreversible => "Irreversible action",
            Self::SupersedesActed => "Replaces shipped work",
            Self::Budget => "Budget exhausted",
            Self::Stuck => "Taste call or deadlock",
            Self::DepCollapsed => "A foundation moved",
            Self::ReopenedAfterAction => "Shipped work contradicted",
            Self::ActionUnconfirmed => "Action never reported back",
            Self::RerunHeld => "Rerun waiting on budget",
            Self::TaskTotal => "Whole task paused",
            Self::IntentChanged => "Intent changed under shipped work",
            Self::TaskPaused => "Paused on a cost limit",
        }
    }
}

/// One evidence item, as BBP's claim card shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    /// The evidence message.
    pub msg: u64,
    /// Who posted it.
    pub author: String,
    /// `Support` or `Counter`.
    pub direction: String,
    /// `Cited`, `Executed`, and so on.
    pub kind: String,
    /// Its signed contribution to the score.
    pub delta: i64,
    /// The claim epoch it was posted in.
    pub epoch: u32,
}

/// A claim, as BBP's claim card shows it. Fields Mill Board does not use are ignored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claim {
    /// The claim message id (`msg:<id>` on the host).
    pub id: u64,
    /// The claim text.
    pub text: String,
    /// BBP status name.
    pub status: String,
    /// Current-epoch score.
    pub score: i64,
    /// `p` as a whole percent.
    pub confidence: u32,
    /// Reopen count.
    pub epoch: u32,
    /// Budget units spent.
    pub spent: u64,
    /// Budget units available.
    pub budget: u64,
    /// Every evidence item, all epochs.
    #[serde(default)]
    pub evidence: Vec<Evidence>,
    /// For a question: its candidate answers.
    #[serde(default)]
    pub candidates: Vec<u64>,
}

/// One open escalation, exactly as `bbp inbox --json` reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Escalation {
    /// `<task>/<subject>/<seq>`: stable while open; a newer escalation of the same claim differs.
    pub id: String,
    /// BBP task id.
    pub task: String,
    /// The task's brief artifact, which a Refine answer refs.
    #[serde(default)]
    pub brief: Option<u64>,
    /// The task revision this was read at. Every action is fenced on it.
    pub rev: u64,
    /// Log position of the escalating event, within its task.
    pub seq: u64,
    /// Why the human is asked.
    pub reason: Reason,
    /// The claim; `None` for a task total.
    pub claim: Option<Claim>,
    /// `Low`, `Medium` or `High`.
    pub stakes: Option<String>,
    /// `Some(false)` for an irreversible claim.
    pub reversible: Option<bool>,
    /// The two strongest support and two strongest counter items of the current epoch.
    #[serde(default)]
    pub evidence: Vec<Evidence>,
    /// For a stuck question: every live candidate, side by side.
    #[serde(default)]
    pub candidates: Vec<Claim>,
    /// `ActionUnconfirmed`: the mark to reconcile.
    pub mark: Option<u64>,
    /// `RerunHeld`: the candidate artifact waiting on budget.
    pub candidate: Option<u64>,
    /// The claim and candidates shown that were acted on. `None` when the host does
    /// not report it (older than #1420): then no decision that could drop work is sent.
    #[serde(default)]
    pub acted: Option<Vec<Acted>>,
    /// A task total: each exhausted budget field, empty once they are all funded again.
    /// `None` when the host does not report it (older than #1420).
    #[serde(default)]
    pub totals: Option<Vec<Total>>,
    /// `TaskPaused`: why the host paused it. A ceiling pause recurs on resume
    /// unless that ceiling was raised.
    #[serde(default)]
    pub pause: Option<Pause>,
    /// The erosion gate's detail on an escalated claim (#704, D91): the bin's
    /// line from the report that failed it, or else from the latest erosion
    /// run, so `unmeasured` code shows. Display only and passed through as
    /// the host wrote it, so a change to its shape can't break the inbox read.
    #[serde(default)]
    pub erosion: Option<Value>,
}

/// Why the host paused a task (#598).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pause {
    /// `SpendCeiling`, `CallCeiling` or `Breaker`.
    pub reason: String,
    /// The host's account of it.
    pub detail: String,
}

/// A claim that was acted on, with the references of its live action marks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Acted {
    /// The claim id.
    pub claim: u64,
    /// What was done (a PR, a deploy id).
    pub marks: Vec<String>,
}

/// One exhausted task budget field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Total {
    /// BBP's field name, e.g. `Messages` or `ReadsPerTurn`.
    pub field: String,
    /// Spent so far.
    pub spent: u64,
    /// The current limit.
    pub limit: u64,
    /// One allowance: the limit the task opened with.
    pub allowance: u64,
}

impl Total {
    /// The `bbp human extend` that adds exactly one allowance.
    fn extend(&self) -> Result<Step, InboxError> {
        let limit = self
            .limit
            .checked_add(self.allowance)
            .ok_or_else(|| InboxError::Invalid(format!("{} limit would overflow", self.field)))?;
        Ok(Step {
            verb: "extend",
            args: vec![snake(&self.field), limit.to_string()],
        })
    }
}

/// `ReadsPerTurn` -> `reads_per_turn`, the host CLI's field names.
fn snake(field: &str) -> String {
    let mut out = String::new();
    for (i, ch) in field.chars().enumerate() {
        if ch.is_ascii_uppercase() && i > 0 {
            out.push('_');
        }
        out.push(ch.to_ascii_lowercase());
    }
    out
}

impl Escalation {
    /// The acted-on claims `a` would drop: what needs a Remediate card.
    ///
    /// # Errors
    /// [`InboxError::Unsupported`] when `a` could drop a claim and the host does not
    /// report acted-on work: the decision fails closed rather than lose its card.
    pub fn dropped_acted(&self, a: &Action) -> Result<Vec<&Acted>, InboxError> {
        let claim = self.claim.as_ref().map(|c| c.id);
        let drops = |x: &Acted| match (a, self.reason) {
            (
                Action::Reject { .. },
                Reason::RerunHeld
                | Reason::ActionUnconfirmed
                | Reason::TaskTotal
                | Reason::TaskPaused,
            ) => false,
            (Action::Reject { .. }, _) => Some(x.claim) == claim,
            (Action::Choose { chosen }, _) => x.claim != *chosen && Some(x.claim) != claim,
            _ => false,
        };
        let could_drop = match (a, self.reason) {
            (Action::Reject { .. }, r) => !matches!(
                r,
                Reason::RerunHeld
                    | Reason::ActionUnconfirmed
                    | Reason::TaskTotal
                    | Reason::TaskPaused
            ),
            (Action::Choose { .. }, _) => true,
            _ => false,
        };
        match &self.acted {
            Some(acted) => Ok(acted.iter().filter(|x| drops(x)).collect()),
            None if could_drop => Err(InboxError::Unsupported(
                "the bbp host does not report acted-on work (it predates #1420), so a drop could lose its Remediate card",
            )),
            None => Ok(vec![]),
        }
    }

    fn brief(&self) -> Result<String, InboxError> {
        self.brief
            .map(|b| format!("art:{b}"))
            .ok_or_else(|| missing(self, "brief"))
    }
}

/// What the human decides.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Action {
    /// Do it, keep it, or spend exactly one more base allowance.
    Approve,
    /// Drop it. A task total needs a `reason`; nothing else reads one.
    Reject {
        /// Why, for a task cancel.
        #[serde(default)]
        reason: Option<String>,
    },
    /// Taste call: accept candidate `chosen` and drop the others.
    Choose {
        /// The chosen candidate's claim id.
        chosen: u64,
    },
    /// Clarify the intent and let the lenses judge again: a human answer
    /// that refs the brief (C-R10), then a reopen of the claim or a resume
    /// of the task.
    Refine {
        /// The clarification, 1 to [`NOTE_MAX`] characters.
        note: String,
    },
}

/// One `bbp human VERB ARGS...` command. The adapter adds the fence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Step {
    /// The human verb.
    pub verb: &'static str,
    /// Its positional arguments.
    pub args: Vec<String>,
}

impl Step {
    fn new(verb: &'static str, args: &[&str]) -> Self {
        Self {
            verb,
            args: args.iter().map(|a| (*a).to_owned()).collect(),
        }
    }
}

/// BBP's answer to one command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum Response {
    /// Accepted (`Ok`, `Posted`, `Marked`, ...).
    Accepted(Value),
    /// Refused; nothing of this command was written.
    Rejected {
        /// BBP's rejection code, e.g. `StaleRev`.
        code: String,
        /// Its explanation.
        detail: String,
    },
}

impl Response {
    /// Read the JSON a `bbp human` command prints.
    #[must_use]
    pub fn from_json(v: Value) -> Self {
        let Some(r) = v.get("Rejected") else {
            return Self::Accepted(v);
        };
        let field = |k: &str| {
            r.get(k)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        Self::Rejected {
            code: field("code"),
            detail: field("detail"),
        }
    }
}

/// What the first command of an action would write, computed by BBP's pure
/// engine at the escalation's revision. Later commands depend on the first
/// committing, so they are listed, not previewed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DryRun {
    /// The previewed command.
    pub step: Step,
    /// What BBP would answer.
    pub response: Response,
    /// The events it would append.
    pub events: Vec<Value>,
    /// The commands that would follow it.
    pub then: Vec<Step>,
}

/// How an action ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum Outcome {
    /// Every command committed.
    Done,
    /// Command `step` was refused; the ones before it committed.
    Refused {
        /// Index of the refused command; `0` means nothing was written.
        step: usize,
        /// BBP's rejection code.
        code: String,
        /// Its explanation.
        detail: String,
    },
}

/// Why an inbox call failed before or outside BBP's own refusals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InboxError {
    /// The action does not fit this escalation; nothing was sent.
    Invalid(String),
    /// Mill Board cannot express this yet without a host change; nothing was sent.
    Unsupported(&'static str),
    /// Running or reading `bbp` failed.
    Host(String),
}

impl fmt::Display for InboxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(why) => write!(f, "invalid inbox action: {why}"),
            Self::Unsupported(why) => write!(f, "not supported yet: {why}"),
            Self::Host(why) => write!(f, "bbp host: {why}"),
        }
    }
}

impl std::error::Error for InboxError {}

/// Pending human decisions, read from BBP. The adapter does the I/O.
pub trait Escalations: Send + Sync {
    /// The open escalations of `tasks`: irreversible first, then by task in
    /// the order given, oldest first within a task (BBP's order).
    ///
    /// # Errors
    /// [`InboxError::Host`] if the host cannot be read.
    fn open(&self, tasks: &[String]) -> Result<Vec<Escalation>, InboxError>;

    /// What `a` would do to `e`, without writing anything.
    ///
    /// # Errors
    /// The [`plan`] errors, or [`InboxError::Host`].
    fn preview(&self, e: &Escalation, a: &Action) -> Result<DryRun, InboxError>;

    /// Do `a` to `e`, answered by `by` (recorded in BBP's op id).
    ///
    /// # Errors
    /// The [`plan`] errors, or [`InboxError::Host`].
    fn act(&self, e: &Escalation, a: &Action, by: &str) -> Result<Outcome, InboxError>;
}

/// The `bbp human` commands that carry out `a` on `e`, in order (design §4).
///
/// # Errors
/// [`InboxError::Invalid`] if the action does not fit the escalation, and
/// [`InboxError::Unsupported`] where the host does not yet report what the
/// command needs.
pub fn plan(e: &Escalation, a: &Action) -> Result<Vec<Step>, InboxError> {
    if e.reason.is_task() {
        return plan_task(e, a);
    }
    let claim = e
        .claim
        .as_ref()
        .ok_or_else(|| InboxError::Invalid(format!("{} has no claim", e.id)))?;
    let c = format!("msg:{}", claim.id);
    e.dropped_acted(a)?;
    let steps = match (a, e.reason) {
        (Action::Approve, Reason::Budget) => vec![
            Step::new("extend-claim", &[&c]),
            Step::new("claim", &[&c, "reopen"]),
        ],
        (Action::Approve, Reason::RerunHeld) => vec![Step::new("extend-claim", &[&c])],
        (Action::Approve, Reason::ActionUnconfirmed) => {
            vec![Step::new("action-result", &[&mark(e)?, "done"])]
        }
        (Action::Approve, _) => vec![Step::new("claim", &[&c, "accept"])],
        (Action::Reject { .. }, Reason::RerunHeld) => {
            let k = e.candidate.ok_or_else(|| missing(e, "candidate"))?;
            vec![Step::new("withdraw-rerun", &[&format!("art:{k}")])]
        }
        (Action::Reject { .. }, Reason::ActionUnconfirmed) => {
            vec![Step::new("action-result", &[&mark(e)?, "aborted"])]
        }
        (Action::Reject { .. }, _) => vec![Step::new("claim", &[&c, "drop"])],
        (Action::Choose { chosen }, Reason::Stuck) => {
            if !e.candidates.iter().any(|k| k.id == *chosen) {
                return Err(InboxError::Invalid(format!(
                    "msg:{chosen} is not a live candidate of {c}"
                )));
            }
            vec![Step::new("choose", &[&c, &format!("msg:{chosen}")])]
        }
        (Action::Choose { .. }, _) => {
            return Err(InboxError::Invalid(
                "choose is for a stuck question with candidates".into(),
            ));
        }
        (Action::Refine { note }, _) => vec![
            Step::new("refine", &[&c, &e.brief()?, free_text(note)?]),
            Step::new("claim", &[&c, "reopen"]),
        ],
    };
    Ok(steps)
}

/// A paused task. Approve and Refine both continue it, so both add one
/// allowance to each exhausted total before `resume`; Refine first posts
/// the clarification.
fn plan_task(e: &Escalation, a: &Action) -> Result<Vec<Step>, InboxError> {
    // An empty list means every total is funded again (an earlier Approve's extends
    // committed and its resume did not): resume adds nothing, so a retry completes it.
    let extend_and_resume = || -> Result<Vec<Step>, InboxError> {
        let totals = e.totals.as_ref().ok_or(InboxError::Unsupported(
            "the bbp host does not report task totals (it predates #1420)",
        ))?;
        let mut steps = totals
            .iter()
            .map(Total::extend)
            .collect::<Result<Vec<_>, _>>()?;
        steps.push(Step::new("resume", &["open"]));
        Ok(steps)
    };
    match a {
        Action::Reject { reason: Some(r) } => Ok(vec![Step::new("cancel", &[free_text(r)?])]),
        Action::Reject { reason: None } => Err(InboxError::Invalid(
            "cancelling a task needs a reason".into(),
        )),
        Action::Approve => extend_and_resume(),
        Action::Refine { note } => {
            let mut steps = vec![Step::new(
                "refine",
                &["task", &e.brief()?, free_text(note)?],
            )];
            steps.extend(extend_and_resume()?);
            Ok(steps)
        }
        Action::Choose { .. } => Err(InboxError::Invalid(
            "choose is for a stuck question with candidates".into(),
        )),
    }
}

fn mark(e: &Escalation) -> Result<String, InboxError> {
    e.mark
        .map(|m| format!("mark:{m}"))
        .ok_or_else(|| missing(e, "mark"))
}

fn missing(e: &Escalation, what: &str) -> InboxError {
    InboxError::Invalid(format!("{} has no {what}", e.id))
}

/// A note or reason, trimmed: 1 to [`NOTE_MAX`] characters, and not starting
/// with `--`, which the host's argument parser would read as a flag.
fn free_text(text: &str) -> Result<&str, InboxError> {
    let text = text.trim();
    let n = text.chars().count();
    if n == 0 || n > NOTE_MAX {
        return Err(InboxError::Invalid(format!(
            "a note or reason is 1 to {NOTE_MAX} characters"
        )));
    }
    if text.starts_with("--") {
        return Err(InboxError::Invalid(
            "a note or reason cannot start with --".into(),
        ));
    }
    Ok(text)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// The JSON `bbp inbox --json` printed for an irreversible claim (host #1391).
    const SAMPLE: &str = r#"[{"id":"C1/msg:1/14","task":"C1","rev":16,"seq":14,"reason":"Irreversible","claim":{"id":1,"text":"ship it","domains":["d"],"status":"Escalated","score":12,"confidence":96,"epoch":1,"spent":2,"budget":16,"evidence":[{"msg":2,"author":"a","direction":"Support","kind":"Cited","delta":6,"epoch":1,"scoring":true}],"deps":[],"candidates":[],"sole_candidate":false,"current":null,"failing":[],"tests_changed":[],"endorsement":null},"stakes":"Low","reversible":false,"evidence":[{"msg":2,"author":"a","direction":"Support","kind":"Cited","delta":6,"epoch":1,"scoring":true}],"candidates":[],"mark":null,"candidate":null,"brief":5,"acted":[],"totals":[]}]"#;

    fn esc(reason: Reason) -> Escalation {
        let mut e: Vec<Escalation> = serde_json::from_str(SAMPLE).unwrap();
        let mut e = e.remove(0);
        e.reason = reason;
        e
    }

    fn verbs(e: &Escalation, a: &Action) -> Vec<String> {
        plan(e, a)
            .unwrap()
            .into_iter()
            .map(|s| format!("{} {}", s.verb, s.args.join(" ")))
            .collect()
    }

    #[test]
    fn the_host_json_reads() {
        let e = esc(Reason::Irreversible);
        assert_eq!(e.rev, 16);
        assert_eq!(e.reversible, Some(false));
        let c = e.claim.unwrap();
        assert_eq!((c.id, c.confidence, c.spent, c.budget), (1, 96, 2, 16));
        assert_eq!(e.evidence.len(), 1);
        assert_eq!(
            e.erosion, None,
            "a host older than #1519 has no erosion detail"
        );
    }

    #[test]
    fn the_erosion_detail_passes_through_as_the_host_wrote_it() {
        let detail = r#"{"report":9,"failing":true,"error":null,"line":{"unmeasured":["tool.py"],"future":1}}"#;
        let json = SAMPLE.replacen(
            r#""totals":[]"#,
            &format!(r#""totals":[],"erosion":{detail}"#),
            1,
        );
        let rows: Vec<Escalation> = serde_json::from_str(&json).unwrap();
        let want: Value = serde_json::from_str(detail).unwrap();
        assert_eq!(rows[0].erosion.as_ref(), Some(&want));
        assert_eq!(serde_json::to_value(&rows[0]).unwrap()["erosion"], want);
    }

    #[test]
    fn an_intent_change_is_approved_by_accept_and_rejected_by_drop() {
        let e = esc(Reason::IntentChanged);
        assert_eq!(verbs(&e, &Action::Approve), ["claim msg:1 accept"]);
        assert_eq!(
            verbs(&e, &Action::Reject { reason: None }),
            ["claim msg:1 drop"]
        );
    }

    #[test]
    fn a_host_pause_reads_and_resumes_open() {
        // `bbp inbox` JSON for a `task_paused` row (rusty_bbp_host inbox.rs).
        let json = r#"[{"id":"C1/task/9","task":"C1","brief":0,"rev":12,"seq":9,
            "reason":"TaskPaused","claim":null,"stakes":null,"reversible":null,
            "evidence":[],"candidates":[],"mark":null,"candidate":null,"acted":[],
            "totals":[],"pause":{"reason":"CallCeiling","detail":"400 of 400 calls"}}]"#;
        let mut rows: Vec<Escalation> = serde_json::from_str(json).unwrap();
        let e = rows.remove(0);
        assert_eq!(e.reason, Reason::TaskPaused);
        assert_eq!(
            e.pause.as_ref().map(|p| p.reason.as_str()),
            Some("CallCeiling")
        );
        assert_eq!(verbs(&e, &Action::Approve), ["resume open"]);
        assert_eq!(
            verbs(
                &e,
                &Action::Reject {
                    reason: Some("too dear".into())
                }
            ),
            ["cancel too dear"]
        );
        assert!(
            e.dropped_acted(&Action::Reject { reason: None })
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn approve_maps_per_reason() {
        let approve = |r| verbs(&esc(r), &Action::Approve);
        assert_eq!(approve(Reason::Irreversible), ["claim msg:1 accept"]);
        assert_eq!(approve(Reason::DepCollapsed), ["claim msg:1 accept"]);
        assert_eq!(approve(Reason::ReopenedAfterAction), ["claim msg:1 accept"]);
        assert_eq!(
            approve(Reason::Budget),
            ["extend-claim msg:1", "claim msg:1 reopen"],
            "one base allowance, never a typed amount"
        );
        assert_eq!(approve(Reason::RerunHeld), ["extend-claim msg:1"]);
        let mut e = esc(Reason::ActionUnconfirmed);
        e.mark = Some(7);
        assert_eq!(verbs(&e, &Action::Approve), ["action-result mark:7 done"]);
    }

    #[test]
    fn reject_maps_per_reason() {
        let reject = Action::Reject { reason: None };
        assert_eq!(verbs(&esc(Reason::Stuck), &reject), ["claim msg:1 drop"]);
        let mut e = esc(Reason::RerunHeld);
        e.candidate = Some(4);
        assert_eq!(verbs(&e, &reject), ["withdraw-rerun art:4"]);
        let mut e = esc(Reason::ActionUnconfirmed);
        e.mark = Some(7);
        assert_eq!(verbs(&e, &reject), ["action-result mark:7 aborted"]);
    }

    #[test]
    fn a_missing_mark_or_candidate_sends_nothing() {
        assert!(matches!(
            plan(&esc(Reason::ActionUnconfirmed), &Action::Approve),
            Err(InboxError::Invalid(_))
        ));
        assert!(matches!(
            plan(&esc(Reason::RerunHeld), &Action::Reject { reason: None }),
            Err(InboxError::Invalid(_))
        ));
    }

    #[test]
    fn choose_takes_only_a_shown_candidate_of_a_stuck_question() {
        let mut e = esc(Reason::Stuck);
        let mut other = e.claim.clone().unwrap();
        other.id = 5;
        e.candidates = vec![other];
        assert_eq!(
            verbs(&e, &Action::Choose { chosen: 5 }),
            ["choose msg:1 msg:5"]
        );
        assert!(plan(&e, &Action::Choose { chosen: 6 }).is_err());
        assert!(plan(&esc(Reason::Irreversible), &Action::Choose { chosen: 5 }).is_err());
    }

    #[test]
    fn refine_needs_a_note_of_bounded_length() {
        let refine = |n: &str| plan(&esc(Reason::Stuck), &Action::Refine { note: n.into() });
        assert!(refine("  ").is_err());
        assert!(refine(&"x".repeat(NOTE_MAX + 1)).is_err());
        assert!(
            refine("--dry-run").is_err(),
            "the host would read it as a flag"
        );
        assert_eq!(
            verbs(
                &esc(Reason::Stuck),
                &Action::Refine {
                    note: " speed over size ".into()
                }
            ),
            ["refine msg:1 art:5 speed over size", "claim msg:1 reopen"]
        );
        let mut e = esc(Reason::Stuck);
        e.brief = None;
        assert!(
            plan(&e, &Action::Refine { note: "x".into() }).is_err(),
            "an answer must ref the brief"
        );
    }

    #[test]
    fn a_task_total_cancels_extends_by_one_allowance_or_refines() {
        let mut e = esc(Reason::TaskTotal);
        e.claim = None;
        assert_eq!(
            verbs(
                &e,
                &Action::Reject {
                    reason: Some("not worth it".into())
                }
            ),
            ["cancel not worth it"]
        );
        for reason in [None, Some(" ".into())] {
            assert!(matches!(
                plan(&e, &Action::Reject { reason }),
                Err(InboxError::Invalid(_))
            ));
        }
        e.totals = None;
        assert!(
            matches!(plan(&e, &Action::Approve), Err(InboxError::Unsupported(_))),
            "a host that does not report totals cannot be given one allowance"
        );
        e.totals = Some(vec![]);
        assert_eq!(
            verbs(&e, &Action::Approve),
            ["resume open"],
            "totals already funded (an earlier Approve lost its resume): resume adds nothing"
        );
        e.totals = Some(vec![
            Total {
                field: "Messages".into(),
                spent: 41,
                limit: 40,
                allowance: 40,
            },
            Total {
                field: "ReadsPerTurn".into(),
                spent: 31,
                limit: 30,
                allowance: 30,
            },
        ]);
        assert_eq!(
            verbs(&e, &Action::Approve),
            [
                "extend messages 80",
                "extend reads_per_turn 60",
                "resume open"
            ]
        );
        assert_eq!(
            verbs(
                &e,
                &Action::Refine {
                    note: "smaller steps".into()
                }
            ),
            [
                "refine task art:5 smaller steps",
                "extend messages 80",
                "extend reads_per_turn 60",
                "resume open"
            ]
        );
    }

    #[test]
    fn only_a_drop_of_acted_on_work_needs_remediation() {
        let mut e = esc(Reason::ReopenedAfterAction);
        let pr = |claim| Acted {
            claim,
            marks: vec!["pr-7".into()],
        };
        e.acted = Some(vec![pr(1)]);
        let reject = Action::Reject { reason: None };
        assert_eq!(e.dropped_acted(&reject).unwrap(), [&pr(1)]);
        assert!(e.dropped_acted(&Action::Approve).unwrap().is_empty());
        e.reason = Reason::ActionUnconfirmed;
        assert!(
            e.dropped_acted(&reject).unwrap().is_empty(),
            "aborting a mark drops nothing"
        );
        let mut q = esc(Reason::Stuck);
        q.acted = Some(vec![pr(5), pr(6)]);
        assert_eq!(
            q.dropped_acted(&Action::Choose { chosen: 5 }).unwrap(),
            [&pr(6)]
        );
    }

    #[test]
    fn a_host_without_acted_on_metadata_refuses_every_drop() {
        let mut e = esc(Reason::ReopenedAfterAction);
        e.acted = None;
        let reject = Action::Reject { reason: None };
        for a in [&reject, &Action::Choose { chosen: 5 }] {
            assert!(matches!(
                e.dropped_acted(a),
                Err(InboxError::Unsupported(_))
            ));
            assert!(matches!(plan(&e, a), Err(InboxError::Unsupported(_))));
        }
        assert_eq!(verbs(&e, &Action::Approve), ["claim msg:1 accept"]);
        e.reason = Reason::ActionUnconfirmed;
        e.mark = Some(7);
        assert_eq!(verbs(&e, &reject), ["action-result mark:7 aborted"]);
    }

    #[test]
    fn a_rejection_reads_as_refused() {
        let r = Response::from_json(
            serde_json::json!({"Rejected":{"code":"StaleRev","detail":"human view changed"}}),
        );
        assert_eq!(
            r,
            Response::Rejected {
                code: "StaleRev".into(),
                detail: "human view changed".into()
            }
        );
        assert!(matches!(
            Response::from_json(serde_json::json!("Ok")),
            Response::Accepted(_)
        ));
    }
}
