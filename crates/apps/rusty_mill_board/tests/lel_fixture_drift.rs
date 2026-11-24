//! Drift guard for the LEL tab view model (#1416, D174): rusty_term keeps its own
//! parser for the `inbox_list` reply, so both crates read the same golden file.
//! The board serializes `InboxItem` (an `Escalation` plus `label` and `card`), so
//! this rebuilds each item the way `Board::inbox` does and requires the bytes the
//! board would send to equal the fixture. No HTTP or service runs.
//! If this fails, the reply shape changed: update the fixture and the view model.

use rusty_mill_board::board::{InboxCard, InboxItem};
use rusty_mill_board::inbox::{Escalation, Reason};
use serde_json::Value;

const FIXTURE: &str = include_str!("../../../libs/ui/rusty_term/src/gui/testdata/lel_inbox.json");

/// Drops null members, since the fixture may leave out what the board sends as null.
fn without_nulls(v: &Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(
            m.iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| (k.clone(), without_nulls(v)))
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(without_nulls).collect()),
        other => other.clone(),
    }
}

fn card(v: &Value) -> Option<InboxCard> {
    let c = v.get("card").filter(|c| !c.is_null())?;
    Some(InboxCard {
        id: c["id"].as_str()?.parse().ok()?,
        number: c["number"].as_u64(),
        title: c["title"].as_str()?.to_owned(),
        project: c["project"].as_str()?.to_owned(),
    })
}

#[test]
fn the_term_fixture_is_what_the_board_sends() {
    let items: Vec<Value> = serde_json::from_str(FIXTURE).expect("fixture is a JSON list");
    let mut seen = Vec::new();
    for v in &items {
        let escalation: Escalation =
            serde_json::from_value(v.clone()).expect("fixture drifted from Escalation");
        seen.push((escalation.id.clone(), escalation.rev, escalation.reason));
        let item = InboxItem {
            label: escalation.reason.label(),
            card: card(v),
            escalation,
        };
        let sent = serde_json::to_value(&item).expect("InboxItem serializes");
        assert_eq!(
            without_nulls(&sent),
            without_nulls(v),
            "fixture differs from what the board sends"
        );
    }
    assert_eq!(
        seen,
        [
            ("T1/msg:12/3".to_owned(), 41, Reason::Irreversible),
            ("T2/msg:20/5".to_owned(), 9, Reason::Stuck),
            ("T3/task/7".to_owned(), 3, Reason::TaskTotal),
        ]
    );
}
