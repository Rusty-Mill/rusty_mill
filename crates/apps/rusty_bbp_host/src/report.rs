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
    /// Versions that are both a `Conflict` and `Unused` (criterion 4's count).
    pub unused_conflicts: u64,
    /// Each human gate answer and who gave it (the `--as` label of `bbp human`).
    pub gates: Vec<GateReport>,
    /// True unless every gate was answered by the declared owner (`--owner`).
    /// A scripted or sandbox run is self-approved by definition: no person
    /// reviewed what was approved. With no gate answered it is true.
    pub self_approved: bool,
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
/// answers are never the owner's.
pub fn gates(events: &[Event], owner: Option<&str>) -> Vec<GateReport> {
    metrics::gate_answers(events)
        .into_iter()
        .map(|g| {
            let by = human::answered_by(&g.op);
            GateReport {
                gate: g.gate,
                self_approved: owner != Some(by) || by == human::UNLABELLED,
                answered_by: by.to_owned(),
            }
        })
        .collect()
}

/// Read `task`'s log and blobs and compute its [`Report`].
pub fn of(dir: &Path, task: &TaskId, owner: Option<&str>) -> Result<Report, String> {
    let d = open_driver(dir, task)?;
    let events = d.store.events(task, 0).map_err(|e| format!("{e:?}"))?;
    let blob = |b: &BlobRef| d.store.blob_get(&b.sha).ok();
    let overlap = metrics::overlap(&events, &blob);
    let spec_conflicts = spec_metrics::spec_conflicts(&events, &blob);
    let spec_attribution = spec_metrics::spec_attribution(&events, &blob);
    let unused_conflicts = spec_metrics::unused_conflicts(&spec_conflicts, &spec_attribution);
    let unused_spec_versions = spec_attribution.unused_versions();
    let unused_bytes = spec_attribution.unused_bytes();
    let gates = gates(&events, owner);
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
        unused_spec_versions,
        unused_bytes,
    })
}

/// [`of`] as one line of JSON.
pub fn json(dir: &Path, task: &TaskId, owner: Option<&str>) -> Result<String, String> {
    rusty_serde::json::to_string(&of(dir, task, owner)?).map_err(|e| e.to_string())
}

#[cfg(test)]
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

    #[test]
    fn only_the_declared_owner_is_not_self_approved() {
        let log = [approval(), answered("by:owner/1")];
        assert!(gates(&log, None)[0].self_approved, "no owner declared");
        assert!(gates(&log, Some("someone")).iter().all(|g| g.self_approved));
        assert!(!gates(&log, Some("owner"))[0].self_approved);
    }

    #[test]
    fn an_unlabelled_answer_is_never_the_owners() {
        let log = [approval(), answered("auto-1")];
        let g = &gates(&log, Some(human::UNLABELLED))[0];
        assert!((g.answered_by.as_str(), g.self_approved) == ("unlabelled", true));
    }

    #[test]
    fn no_gate_answered_means_nothing_to_vouch_for() {
        assert!(gates(&[], Some("owner")).is_empty());
    }
}
