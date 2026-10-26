use super::*;
use crate::parse;

fn apply(records: &mut Records, body: &str) {
    records.apply(parse(body).expect(body));
}

fn ids(records: &Records) -> Vec<&str> {
    records.iter().map(|(id, _)| id.as_str()).collect()
}

#[test]
fn a_report_replaces_its_record_whole() {
    let mut records = Records::default();
    apply(&mut records, "state=working:app=x:progress=10");
    apply(&mut records, "state=done");
    let (_, record) = records.iter().next().expect("root");
    assert_eq!(
        (record.state, record.app.as_deref(), record.progress),
        (State::Done, None, None)
    );
}

#[test]
fn clear_removes_a_subtree_or_everything() {
    let mut records = Records::default();
    for body in [
        "state=idle",
        "state=working:id=lint",
        "state=blocked:id=lint/js",
        "state=done:id=linter",
        "state=error:id=test",
    ] {
        apply(&mut records, body);
    }
    apply(&mut records, "state=clear:id=lint");
    assert_eq!(ids(&records), ["", "linter", "test"]);
    apply(&mut records, "state=clear");
    assert!(ids(&records).is_empty());
}

#[test]
fn a_prompt_ends_working_and_blocked_records() {
    let mut records = Records::default();
    for body in [
        "state=idle",
        "state=working:id=a",
        "state=blocked:id=b",
        "state=done:id=c",
        "state=error:id=d",
    ] {
        apply(&mut records, body);
    }
    records.prompt();
    assert_eq!(ids(&records), ["", "c", "d"]);
    records.reset();
    assert!(ids(&records).is_empty());
}

#[test]
fn the_least_recently_updated_record_is_evicted() {
    let mut records = Records::default();
    for n in 0..MAX_RECORDS {
        apply(&mut records, &format!("state=idle:id=r{n}"));
    }
    apply(&mut records, "state=done:id=r0"); // updating keeps r0 fresh
    apply(&mut records, "state=done:id=new");
    assert_eq!(records.iter().count(), MAX_RECORDS);
    let kept = ids(&records);
    assert!(kept.contains(&"r0") && kept.contains(&"new"));
    assert!(!kept.contains(&"r1"), "r1 was the oldest");
}

#[test]
fn bodies_round_trip_and_inherit_app() {
    let mut records = Records::default();
    let msg = rusty_base64::encode_standard("Continue?".as_bytes());
    apply(&mut records, "state=working:app=tool:progress=5");
    apply(
        &mut records,
        &format!("state=blocked:id=a/b:kind=permission:msg={msg}"),
    );
    let bodies = records.bodies();
    assert_eq!(
        bodies,
        [
            "state=working:progress=5:app=tool".to_owned(),
            format!("state=blocked:id=a/b:kind=permission:app=tool:msg={msg}"),
        ]
    );
    for body in &bodies {
        assert!(parse(body).is_some(), "{body}");
    }
}

#[test]
fn states_ignore_progress_and_text() {
    let mut records = Records::default();
    apply(&mut records, "state=working:progress=1");
    let before = records.states();
    apply(&mut records, "state=working:progress=2:msg=aGk");
    assert_eq!(records.states(), before);
    apply(&mut records, "state=done");
    assert_ne!(records.states(), before);
}
