use orch_cli::fake::{REPLY_ONE, REPLY_TWO};
use orch_cli::{parse, MAX_BODY_CHARS, MAX_ENTRIES};
use orch_core::board::{Confidence, EntryKind, Verdict};
use orch_core::task::Role;
use orch_core::{EntryId, Ref, TaskId};

fn err(reply: &str) -> String {
    parse(reply, Role::Research)
        .expect_err("rejected")
        .message()
        .to_owned()
}

#[test]
fn accepts_example_reply_one() {
    let out = parse(REPLY_ONE, Role::Research).expect("valid");
    assert_eq!(out.len(), 1);
    assert_eq!(
        out[0].kind,
        EntryKind::Finding {
            confidence: Confidence::High
        }
    );
    assert_eq!(out[0].refs.len(), 2);
    assert_eq!(out[0].refs[0], Ref::Entry(EntryId::from_raw(1)));
    assert!(
        matches!(&out[0].refs[1], Ref::Path(p) if p.as_str() == "crates/orch-core/src/task.rs")
    );
    assert_eq!(out[0].supersedes, None);
}

#[test]
fn accepts_example_reply_two() {
    let out = parse(REPLY_TWO, Role::Research).expect("valid");
    assert_eq!(out.len(), 2);
    assert_eq!(out[1].kind, EntryKind::Question);
    assert!(out[1].refs.is_empty());
}

#[test]
fn accepts_one_fence_wrapper() {
    let fenced = format!("```json\n{REPLY_ONE}\n```");
    assert_eq!(parse(&fenced, Role::Research).expect("valid").len(), 1);
}

#[test]
fn rejects_non_json_and_missing_entries() {
    assert!(err("Sure! Here is my answer.").contains("not JSON"));
    assert!(err(r#"{"findings":[]}"#).contains("no \"entries\""));
    assert!(err(r#"{"entries":[]}"#).contains("zero entries"));
}

#[test]
fn rejects_unknown_kind_and_kind_not_allowed_for_role() {
    assert!(err(r#"{"entries":[{"kind":"poem","body":"x","refs":[]}]}"#).contains("not allowed"));
    assert!(
        err(r#"{"entries":[{"kind":"decision","body":"x","refs":[]}]}"#).contains("not allowed")
    );
    let design = r#"{"entries":[{"kind":"decision","body":"x","refs":[]}]}"#;
    assert!(parse(design, Role::Design)
        .expect_err("agents never settle decisions")
        .message()
        .contains("not allowed"));
}

#[test]
fn rejects_unserved_roles() {
    let unserved = parse(REPLY_ONE, Role::Implement).expect_err("unserved role");
    assert!(
        unserved.is_permanent(),
        "no reply can make Implement servable"
    );
}

fn review_role() -> Role {
    Role::Review {
        target: TaskId::from_raw(1),
    }
}

#[test]
fn review_card_accepts_one_review_with_a_verdict() {
    let approve =
        r#"{"entries":[{"kind":"review","verdict":"approve","body":"Sound.","refs":["E-1"]}]}"#;
    let out = parse(approve, review_role()).expect("valid");
    assert_eq!(
        out[0].kind,
        EntryKind::Review {
            of: TaskId::from_raw(1),
            verdict: Verdict::Approve
        }
    );
    let changes = r#"{"entries":[{"kind":"finding","confidence":"low","body":"Missing test.","refs":[]},{"kind":"review","verdict":"changes_requested","body":"Add a test.","refs":["E-1"]}]}"#;
    let out = parse(changes, review_role()).expect("valid");
    assert_eq!(out.len(), 2);
    assert!(matches!(
        out[1].kind,
        EntryKind::Review {
            verdict: Verdict::ChangesRequested,
            ..
        }
    ));
}

#[test]
fn review_card_rejects_missing_bad_or_duplicate_verdicts() {
    let no_verdict = r#"{"entries":[{"kind":"review","body":"x","refs":[]}]}"#;
    assert!(parse(no_verdict, review_role())
        .expect_err("no verdict")
        .message()
        .contains("verdict"));
    let bad = r#"{"entries":[{"kind":"review","verdict":"lgtm","body":"x","refs":[]}]}"#;
    assert!(parse(bad, review_role())
        .expect_err("bad verdict")
        .message()
        .contains("not approve"));
    // Findings alone are not a review: the card must deliver its verdict.
    assert!(parse(REPLY_ONE, review_role())
        .expect_err("no review")
        .message()
        .contains("must include one review"));
    let two = r#"{"entries":[{"kind":"review","verdict":"approve","body":"x","refs":[]},{"kind":"review","verdict":"approve","body":"y","refs":[]}]}"#;
    assert!(parse(two, review_role())
        .expect_err("two")
        .message()
        .contains("exactly one"));
    let research = r#"{"entries":[{"kind":"review","verdict":"approve","body":"x","refs":[]}]}"#;
    assert!(err(research).contains("not allowed"));
}

#[test]
fn rejects_blank_body_and_missing_body() {
    assert!(err(r#"{"entries":[{"kind":"question","body":"   ","refs":[]}]}"#).contains("blank"));
    assert!(err(r#"{"entries":[{"kind":"question","refs":[]}]}"#)
        .contains("missing string field \"body\""));
}

#[test]
fn rejects_missing_or_bad_confidence() {
    assert!(err(r#"{"entries":[{"kind":"finding","body":"x","refs":[]}]}"#).contains("confidence"));
    assert!(
        err(r#"{"entries":[{"kind":"finding","confidence":"sure","body":"x","refs":[]}]}"#)
            .contains("not low, medium, or high")
    );
}

#[test]
fn rejects_malformed_refs_and_missing_refs() {
    for bad in [
        "E-0",
        "E-x",
        "E1",
        "path:",
        "ftp:host",
        "no-scheme",
        "commit:  ",
    ] {
        let reply = format!(r#"{{"entries":[{{"kind":"question","body":"x","refs":["{bad}"]}}]}}"#);
        assert!(
            parse(&reply, Role::Research).is_err(),
            "{bad} should be rejected"
        );
    }
    assert!(
        err(r#"{"entries":[{"kind":"question","body":"x","refs":[7]}]}"#).contains("not a string")
    );
    assert!(err(r#"{"entries":[{"kind":"question","body":"x"}]}"#).contains("missing \"refs\""));
}

#[test]
fn rejects_over_cap_entry_count_and_body_length() {
    let one = r#"{"kind":"question","body":"x","refs":[]}"#;
    let many = [one; MAX_ENTRIES + 1].join(",");
    assert!(err(&format!(r#"{{"entries":[{many}]}}"#)).contains("exceeds the cap"));
    let at_cap = [one; MAX_ENTRIES].join(",");
    assert_eq!(
        parse(&format!(r#"{{"entries":[{at_cap}]}}"#), Role::Research)
            .expect("at cap")
            .len(),
        MAX_ENTRIES
    );

    let long = "é".repeat(MAX_BODY_CHARS + 1);
    assert!(err(&format!(
        r#"{{"entries":[{{"kind":"question","body":"{long}","refs":[]}}]}}"#
    ))
    .contains("exceeds"));
    let max = "é".repeat(MAX_BODY_CHARS);
    assert!(parse(
        &format!(r#"{{"entries":[{{"kind":"question","body":"{max}","refs":[]}}]}}"#),
        Role::Research
    )
    .is_ok());
}
