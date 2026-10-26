use super::*;

fn set(body: &str) -> (Id, Record) {
    match parse(body) {
        Some(Report::Set(id, record)) => (id, record),
        other => panic!("{body}: {other:?}"),
    }
}

fn b64(text: &str) -> String {
    rusty_base64::encode_standard(text.as_bytes())
}

#[test]
fn a_full_report_parses() {
    let body = format!(
        "state=blocked:id=lint/js:kind=question:progress=30:app=cargo:title={}:msg={}",
        b64("Lint"),
        b64("Continue?")
    );
    let (id, record) = set(&body);
    assert_eq!(id.as_str(), "lint/js");
    assert_eq!(
        record,
        Record {
            state: State::Blocked,
            kind: Some(Kind::Question),
            progress: Some(30),
            app: Some("cargo".into()),
            title: Some("Lint".into()),
            msg: Some("Continue?".into()),
        }
    );
}

#[test]
fn pairs_are_trimmed_skipped_or_overridden() {
    // Spaces trimmed; a pair without `=`, an empty key and an uppercase key
    // are skipped; unknown keys are ignored; the last `app` wins.
    let (id, record) = set(" state = working :junk:=x:APP=y:app=a!b:future=1:app=first:app=last");
    assert!(id.is_root());
    assert_eq!(record.state, State::Working);
    assert_eq!(record.app.as_deref(), Some("last"));
}

#[test]
fn unpadded_base64_is_accepted() {
    let (_, record) = set("state=done:msg=b2s");
    assert_eq!(record.msg.as_deref(), Some("ok"));
}

#[test]
fn clear_needs_no_other_key() {
    assert_eq!(parse("state=clear"), Some(Report::Clear(Id::default())));
    assert_eq!(
        parse("state=clear:id=a/b"),
        Some(Report::Clear(Id("a/b".into())))
    );
}

#[test]
fn reports_without_a_known_state_or_with_a_bad_id_are_ignored() {
    for body in [
        "",
        "app=x",
        "state=sleeping",
        "state=WORKING",
        "state=idle:id=",
        "state=idle:id=a//b",
        "state=idle:id=/a",
        "state=idle:id=a b",
        "state=idle:id=1/2/3/4/5/6/7/8/9",
    ] {
        assert_eq!(parse(body), None, "{body}");
    }
    let long_segment = format!("state=idle:id={}", "a".repeat(33));
    assert_eq!(parse(&long_segment), None);
    let long_id = format!("state=idle:id={}", vec!["a".repeat(31); 5].join("/"));
    assert_eq!(parse(&long_id), None, "over 128 bytes");
}

#[test]
fn bad_text_or_size_discards_the_whole_report() {
    let cases = [
        "state=done:msg=!!!!".to_owned(),
        format!("state=done:msg={}", b64("two\nlines")),
        format!("state=done:title={}", b64("tab\there")),
        format!(
            "state=done:msg={}",
            rusty_base64::encode_standard(&[0xff, 0xfe])
        ),
        format!("state=done:title={}", b64(&"t".repeat(193))),
        format!("state=done:msg={}", b64(&"m".repeat(2049))),
        format!("state=done:app=x:future={}", "z".repeat(4096)),
    ];
    for body in cases {
        assert_eq!(parse(&body), None, "{body:.60}");
    }
    let (_, at_limit) = set(&format!("state=done:msg={}", b64(&"m".repeat(2048))));
    assert_eq!(at_limit.msg.map(|m| m.len()), Some(2048));
}

#[test]
fn soft_fields_fall_back_to_absent() {
    // Unknown kind, out-of-range progress and a bad app are dropped, not
    // fatal; kind needs blocked and progress needs working or blocked.
    let (_, r) = set("state=blocked:kind=coffee:progress=101:app=a/b");
    assert_eq!((r.kind, r.progress, r.app), (None, None, None));
    let (_, r) = set("state=done:kind=auth:progress=50");
    assert_eq!((r.kind, r.progress), (None, None));
    let (_, r) = set(&format!("state=working:progress=0:app={}", "a".repeat(32)));
    assert_eq!(r.progress, Some(0));
    assert!(r.app.is_some());
}
