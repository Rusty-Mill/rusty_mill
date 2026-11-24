//! Drift guard for the LEL tab view model (#1416, D174): rusty_term keeps its own
//! parser for the `inbox_list` reply, so both crates parse the same golden file.
//! If this fails, the reply shape changed: update the fixture and the view model.

use rusty_mill_board::inbox::{Escalation, Reason};

const FIXTURE: &str = include_str!("../../../libs/ui/rusty_term/src/gui/testdata/lel_inbox.json");

#[test]
fn the_term_fixture_still_parses_as_escalations() {
    let list: Vec<Escalation> =
        serde_json::from_str(FIXTURE).expect("fixture drifted from Escalation");
    let seen: Vec<(&str, u64, Reason)> = list
        .iter()
        .map(|e| (e.id.as_str(), e.rev, e.reason))
        .collect();
    assert_eq!(
        seen,
        [
            ("T1/msg:12/3", 41, Reason::Irreversible),
            ("T2/msg:20/5", 9, Reason::Stuck),
            ("T3/task/7", 3, Reason::TaskTotal),
        ]
    );
    assert_eq!(list[1].candidates.len(), 2);
    assert!(list[2].claim.is_none());
}
