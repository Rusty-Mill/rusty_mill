//! `bbp metrics`: the concurrency metrics of one task, read from its log.

use crate::human;
use crate::open_driver;
use rusty_bbp::metrics::{self, GateKind, Metrics, Overlap};
use rusty_bbp::spec_metrics::{self, SpecAttribution, SpecConflicts};
use rusty_bbp::*;
use rusty_serde::Serialize;
use std::path::Path;

/// What `bbp metrics` prints. Wall clock is not here: the log carries no
/// timestamps; `bbp mod` reports `elapsed_ms` for the run.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Report {
    pub task: String,
    pub max_live_turns: u32,
    pub state: String,
    pub metrics: Metrics,
    pub overlap: Overlap,
    /// Each non-approved spec version against the approved one (2b criterion 4).
    pub spec_conflicts: SpecConflicts,
    /// Whether each non-approved spec version's work reached the approved one.
    pub spec_attribution: SpecAttribution,
    /// Versions that are both a `Conflict` and `Unused`: an upper bound for criterion 4.
    pub unused_conflicts: u64,
    /// Each human gate answer and who gave it (the `--as` label of `bbp human`).
    pub gates: Vec<GateReport>,
    /// True unless every gate was answered by the declared owner (`--owner`).
    /// A scripted or sandbox run is self-approved by definition: no person
    /// reviewed what was approved. With no gate answered it is true. On an owned
    /// task a plan or merge approval counts only with a board receipt that
    /// verifies again here (#447).
    pub self_approved: bool,
    /// The task opened with scratch gates (#447): its approvals needed no receipt.
    pub scratch: bool,
    /// `unused_spec_versions` of the 2b gate.
    pub unused_spec_versions: u64,
    /// `unused_share` as `[unused bytes, all spec bytes]`; the gate takes the ratio.
    pub unused_bytes: (u64, u64),
}

/// One human gate answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GateReport {
    pub gate: GateKind,
    pub answered_by: String,
    pub self_approved: bool,
}

/// The gate answers of `events` against the declared `owner`. Unlabelled
/// answers are never the owner's. On an owned task (`scratch` false) a plan or
/// merge approval is the owner's only if `receipted` vouches for the answer:
/// the receipt its op names is stored, verifies under the board key, and is
/// bound to what the log says was approved.
pub fn gates(
    events: &[Event],
    owner: Option<&str>,
    scratch: bool,
    receipted: &dyn Fn(&metrics::GateAnswer) -> bool,
) -> Vec<GateReport> {
    metrics::gate_answers(events)
        .into_iter()
        .map(|g| {
            let by = human::answered_by(&g.op);
            let approval = g.gate != GateKind::Rejection;
            let unsigned = approval && !scratch && !receipted(&g);
            GateReport {
                gate: g.gate,
                self_approved: owner != Some(by) || by == human::UNLABELLED || unsigned,
                answered_by: by.to_owned(),
            }
        })
        .collect()
}

/// Whether answer `g` on `task` names a stored receipt that verifies under `key`, whose
/// approver is the op's label, and that is bound to the task, gate, artifact, run and revision
/// the log records for `g` (a receipt copied onto another answer does not hold).
fn receipt_holds(
    g: &metrics::GateAnswer,
    task: &TaskId,
    key: Option<&[u8; 32]>,
    blob: &dyn Fn(&Sha256) -> Option<Vec<u8>>,
) -> bool {
    let (Some(key), Some(sha), Some(approved)) =
        (key, crate::approval::receipt_of(&g.op), g.approved.as_ref())
    else {
        return false;
    };
    let gate = match g.gate {
        GateKind::PlanApproval => Gate::Plan,
        GateKind::MergeApproval => Gate::Merge,
        GateKind::Rejection => return false,
    };
    blob(&sha)
        .filter(|bytes| Sha256::of(bytes) == sha)
        .and_then(|bytes| crate::approval::verify(key, &bytes).ok())
        .is_some_and(|v| {
            v.receipt().approver == human::answered_by(&g.op)
                && crate::approval::vouches_for(v.receipt(), task, gate, approved)
        })
}

/// Read `task`'s log and blobs and compute its [`Report`].
pub fn of(dir: &Path, task: &TaskId, owner: Option<&str>) -> Result<Report, String> {
    let key = crate::approval::key(crate::approval::BOARD_KEY).ok();
    of_with_key(dir, task, owner, key.as_ref())
}

/// [`of`], verifying receipts under `key` (`None`: no receipt verifies).
pub fn of_with_key(
    dir: &Path,
    task: &TaskId,
    owner: Option<&str>,
    key: Option<&[u8; 32]>,
) -> Result<Report, String> {
    let d = open_driver(dir, task)?;
    let events = d.store.events(task, 0).map_err(|e| format!("{e:?}"))?;
    let blob = |b: &BlobRef| d.store.blob_get(&b.sha).ok();
    let overlap = metrics::overlap(&events, &blob);
    let spec_conflicts = spec_metrics::spec_conflicts(&events, &blob);
    let spec_attribution = spec_metrics::spec_attribution(&events, &blob);
    let unused_conflicts = spec_metrics::unused_conflicts(&spec_conflicts, &spec_attribution);
    let unused_spec_versions = spec_attribution.unused_versions();
    let unused_bytes = spec_attribution.unused_bytes();
    let scratch = d.state.gates == Some(Gates::Scratch);
    let stored = |sha: &Sha256| d.store.blob_get(sha).ok();
    let gates = gates(&events, owner, scratch, &|g| {
        receipt_holds(g, task, key, &stored)
    });
    let self_approved = gates.iter().all(|g| g.self_approved);
    Ok(Report {
        task: task.0.clone(),
        max_live_turns: d.state.max_live_turns,
        state: format!("{:?}", d.state.state),
        metrics: metrics::of(&events),
        overlap,
        spec_conflicts,
        spec_attribution,
        unused_conflicts,
        gates,
        self_approved,
        scratch,
        unused_spec_versions,
        unused_bytes,
    })
}

/// [`of`] as one line of JSON.
pub fn json(dir: &Path, task: &TaskId, owner: Option<&str>) -> Result<String, String> {
    rusty_serde::json::to_string(&of(dir, task, owner)?).map_err(|e| e.to_string())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn answered(op: &str) -> Event {
        Event::OpAccepted {
            principal: PrincipalId("human".into()),
            op: OpId(op.into()),
            payload: Sha256([0; 32]),
            response: Response::Ok,
        }
    }

    fn approval() -> Event {
        Event::HumanApproval {
            gate: Gate::Plan,
            subject: ArtId(2),
            run: None,
        }
    }

    /// The pre-#447 reading: a scratch task, where a label is the answer.
    fn scratch(log: &[Event], owner: Option<&str>) -> Vec<GateReport> {
        gates(log, owner, true, &|_| false)
    }

    #[test]
    fn only_the_declared_owner_is_not_self_approved() {
        let log = [approval(), answered("by:owner/1")];
        assert!(scratch(&log, None)[0].self_approved, "no owner declared");
        assert!(scratch(&log, Some("someone"))
            .iter()
            .all(|g| g.self_approved));
        assert!(!scratch(&log, Some("owner"))[0].self_approved);
    }

    #[test]
    fn an_owned_approval_counts_only_with_a_receipt_that_verifies() {
        let log = [approval(), answered("by:owner/1")];
        assert!(
            gates(&log, Some("owner"), false, &|_| false)[0].self_approved,
            "a label alone"
        );
        assert!(!gates(&log, Some("owner"), false, &|_| true)[0].self_approved);
    }

    #[test]
    fn a_stored_receipt_is_checked_again_against_the_key_and_label() {
        use crate::approval::tests::{receipt, sign, signer};
        let k = signer();
        let file = sign(&k, &receipt("plan", None));
        let sha = Sha256::of(&file);
        let op = OpId(format!("by:nano/rcpt:{}", sha.hex()));
        let stored = |s: &Sha256| (*s == sha).then(|| file.clone());
        assert!(receipt_holds(&op, Some(k.public_key()), &stored));
        assert!(!receipt_holds(&op, None, &stored), "no key installed");
        let forged = OpId(format!("by:evil/rcpt:{}", sha.hex()));
        assert!(
            !receipt_holds(&forged, Some(k.public_key()), &stored),
            "label is not the approver"
        );
        let other = rusty_pk::ed25519::SigningKey::from_seed(&[9; 32]).unwrap();
        assert!(
            !receipt_holds(&op, Some(other.public_key()), &stored),
            "another key"
        );
        assert!(
            !receipt_holds(&op, Some(k.public_key()), &|_| None),
            "receipt missing"
        );
    }

    #[test]
    fn an_unlabelled_answer_is_never_the_owners() {
        let log = [approval(), answered("auto-1")];
        let g = &scratch(&log, Some(human::UNLABELLED))[0];
        assert!((g.answered_by.as_str(), g.self_approved) == ("unlabelled", true));
    }

    #[test]
    fn no_gate_answered_means_nothing_to_vouch_for() {
        assert!(scratch(&[], Some("owner")).is_empty());
    }

    #[test]
    fn a_receipt_holds_only_on_the_answer_the_log_says_it_approved() {
        use crate::approval::tests::{receipt, sign, signer};
        let k = signer();
        let file = sign(&k, &receipt("plan", None));
        let v = crate::approval::verify(k.public_key(), &file).unwrap();
        let op = crate::approval::op(&v);
        let blob = |sha: &Sha256| (*sha == Sha256::of(v.bytes())).then(|| v.bytes().to_vec());
        let answer = |gate, subject, rev| metrics::GateAnswer {
            gate,
            op: op.clone(),
            approved: Some(metrics::Approved {
                subject: ArtId(subject),
                run: None,
                rev: Rev(rev),
            }),
        };
        let t1 = TaskId("T1".into());
        let holds =
            |g: &metrics::GateAnswer, t: &TaskId| receipt_holds(g, t, Some(k.public_key()), &blob);
        assert!(holds(&answer(GateKind::PlanApproval, 2, 9), &t1));
        assert!(
            !holds(&answer(GateKind::PlanApproval, 2, 9), &TaskId("T2".into())),
            "task"
        );
        assert!(
            !holds(&answer(GateKind::PlanApproval, 2, 10), &t1),
            "revision"
        );
        assert!(
            !holds(&answer(GateKind::PlanApproval, 3, 9), &t1),
            "artifact"
        );
        assert!(!holds(&answer(GateKind::MergeApproval, 2, 9), &t1), "gate");
    }
}
