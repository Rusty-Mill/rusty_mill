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
    /// Clarify the intent and let the lenses judge again: a human `answer`
    /// to the claim, then a reopen. Known limit: claims mode on the host
    /// still refuses human answers (`WrongMode`), so today BBP refuses the
    /// first command and nothing is written.
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
    if e.reason == Reason::TaskTotal {
        return plan_task(a);
    }
    let claim = e
        .claim
        .as_ref()
        .ok_or_else(|| InboxError::Invalid(format!("{} has no claim", e.id)))?;
    let c = format!("msg:{}", claim.id);
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
            Step::new("answer", &[&c, free_text(note)?]),
            Step::new("claim", &[&c, "reopen"]),
        ],
    };
    Ok(steps)
}

fn plan_task(a: &Action) -> Result<Vec<Step>, InboxError> {
    match a {
        Action::Reject { reason: Some(r) } => Ok(vec![Step::new("cancel", &[free_text(r)?])]),
        Action::Reject { reason: None } => Err(InboxError::Invalid(
            "cancelling a task needs a reason".into(),
        )),
        Action::Approve => Err(InboxError::Unsupported(
            "bbp inbox does not report which task total was crossed, so one allowance cannot be computed",
        )),
        Action::Refine { .. } => Err(InboxError::Unsupported(
            "bbp human answer needs a message to reply to; a task total has none",
        )),
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
    const SAMPLE: &str = r#"[{"id":"C1/msg:1/14","task":"C1","rev":16,"seq":14,"reason":"Irreversible","claim":{"id":1,"text":"ship it","domains":["d"],"status":"Escalated","score":12,"confidence":96,"epoch":1,"spent":2,"budget":16,"evidence":[{"msg":2,"author":"a","direction":"Support","kind":"Cited","delta":6,"epoch":1,"scoring":true}],"deps":[],"candidates":[],"sole_candidate":false,"current":null,"failing":[],"tests_changed":[],"endorsement":null},"stakes":"Low","reversible":false,"evidence":[{"msg":2,"author":"a","direction":"Support","kind":"Cited","delta":6,"epoch":1,"scoring":true}],"candidates":[],"mark":null,"candidate":null}]"#;

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
            ["answer msg:1 speed over size", "claim msg:1 reopen"]
        );
    }

    #[test]
    fn a_task_total_cancels_with_a_reason_and_defers_the_rest() {
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
        assert!(matches!(
            plan(&e, &Action::Approve),
            Err(InboxError::Unsupported(_))
        ));
        assert!(matches!(
            plan(&e, &Action::Refine { note: "x".into() }),
            Err(InboxError::Unsupported(_))
        ));
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
