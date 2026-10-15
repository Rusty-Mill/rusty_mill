//! Reproductions of the independent review's findings on the run tooling,
//! built on real `claude -p --output-format stream-json --verbose` captures in
//! `fixtures/stream-json/` (captured 2026-10-15 with Claude Code 2.1.300).
//! Each test states what a correct tool does; they were red before the fixes.

use rusty_bbp::*;
use rusty_bbp_host::moderator::log_path;
use rusty_bbp_host::tokens::{self, Criteria};
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> String {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/stream-json")
        .join(format!("{name}.jsonl"));
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("bbp-review-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("mkdir");
    d
}

/// Sonnet 5.5 and Haiku 5.5 as the price page lists them (Haiku: input 0.10,
/// output 0.50, 5m write 0.125, 1h write 0.20, read 0.01).
const BOTH: &str = r#"{"prices_as_of":"t","models":{
  "claude-sonnet-5-5":{"input":2.0,"output":10.0,"cache_creation_5m":2.5,"cache_creation_1h":4.0,"cache_read":0.10},
  "claude-haiku-5-5":{"input":0.10,"output":0.50,"cache_creation_5m":0.125,"cache_creation_1h":0.20,"cache_read":0.01}}}"#;
const SONNET_ONLY: &str = r#"{"prices_as_of":"t","models":{
  "claude-sonnet-5-5":{"input":2.0,"output":10.0,"cache_creation_5m":2.5,"cache_creation_1h":4.0,"cache_read":0.10}}}"#;

fn criteria(tag: &str, json: &str) -> Criteria {
    let f = scratch(tag).join("criteria.json");
    std::fs::write(&f, json).expect("write");
    tokens::read_criteria(&f).expect("criteria").0
}

/// One turn per `(role, log text)`, in a scratch dir.
fn tokens_of(
    tag: &str,
    logs: &[(&str, String)],
    c: &Criteria,
) -> Result<tokens::TaskTokens, String> {
    let dir = scratch(tag);
    let task = TaskId("T".into());
    let mut events = Vec::new();
    for (i, (role, text)) in logs.iter().enumerate() {
        let turn = TurnId(i as u64 + 1);
        let p = log_path(&dir, &task, turn);
        std::fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
        std::fs::write(&p, text).expect("write log");
        events.push(Event::TurnGranted {
            role: RoleId((*role).into()),
            turn,
            kind: TurnKind::Default,
            deadline: Time(0),
            return_to: None,
            slice: None,
        });
    }
    let r = tokens::of(&dir, &task, &events, Some(c));
    let _ = std::fs::remove_dir_all(&dir);
    r
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 + 1e-9 * b.abs()
}

// ---- finding 1: output tokens on an assistant line are a placeholder

#[test]
fn output_tokens_are_priced_from_the_result_line_not_the_assistant_placeholder() {
    // The real shape, with the result line reporting a long reply: 100000
    // output tokens at $10/M is $1.00, against 4 on the assistant line.
    let text = fixture("completed-default-ttl")
        .replace("\"outputTokens\":4", "\"outputTokens\":100000")
        .replace(
            "\"total_cost_usd\":0.0086289",
            "\"total_cost_usd\":1.0085889",
        )
        .replace("\"costUSD\":0.0086289", "\"costUSD\":1.0085889");
    let t = tokens_of("f1", &[("planner", text)], &criteria("f1c", SONNET_ONLY)).expect("priced");
    assert!(near(t.usd.expect("known"), 1.0085889), "{:?}", t.usd);
}

// ---- finding 2: subagent models are only in modelUsage

#[test]
fn a_subagent_model_without_a_price_is_refused() {
    let err = tokens_of(
        "f2a",
        &[("coder", fixture("completed-with-subagent"))],
        &criteria("f2ac", SONNET_ONLY),
    )
    .unwrap_err();
    assert!(err.contains("claude-haiku-5-5"), "{err}");
}

#[test]
fn a_subagents_cost_is_counted_when_its_model_is_priced() {
    let t = tokens_of(
        "f2b",
        &[("coder", fixture("completed-with-subagent"))],
        &criteria("f2bc", BOTH),
    )
    .expect("priced");
    // Reported by the harness: 0.030979375. Priced here at the 1-hour write
    // rate for every cache write, so never below it.
    let usd = t.usd.expect("known");
    assert!(usd >= 0.030979375 - 1e-9 && usd < 0.0312, "{usd}");
}

// ---- finding 3 and 4: no usable result line means unknown cost, not zero

#[test]
fn a_turn_that_printed_no_result_has_an_unknown_cost() {
    let only_assistant: String = fixture("completed-default-ttl")
        .lines()
        .filter(|l| l.contains("\"type\":\"assistant\""))
        .map(|l| format!("{l}\n"))
        .collect();
    let t = tokens_of(
        "f3a",
        &[("planner", only_assistant)],
        &criteria("f3ac", SONNET_ONLY),
    )
    .expect("reported");
    assert_eq!(
        t.usd, None,
        "a killed harness's cost is unknown, not smaller"
    );
    assert!(!t.turns[0].complete);
}

#[test]
fn an_empty_or_plain_text_log_has_an_unknown_cost() {
    for (tag, text) in [("f3b", ""), ("f3c", "hello, no json here\n")] {
        let t = tokens_of(
            tag,
            &[("planner", text.into())],
            &criteria(tag, SONNET_ONLY),
        )
        .expect("reported");
        assert_eq!(t.usd, None, "{tag}");
        assert!(!t.turns[0].complete, "{tag}");
    }
}

#[test]
fn a_crash_result_with_zeroed_usage_is_not_a_complete_turn() {
    for name in ["crash-unrecognized-model", "interrupted-sigint"] {
        let t = tokens_of(
            name,
            &[("planner", fixture(name))],
            &criteria(name, SONNET_ONLY),
        )
        .expect("reported");
        assert!(!t.turns[0].complete, "{name}");
        assert_eq!(t.usd, None, "{name}");
    }
}

#[test]
fn the_last_result_line_is_the_one_used() {
    // Two real results in one log, as when background work starts a new turn.
    let both = format!(
        "{}{}",
        fixture("completed-default-ttl"),
        fixture("completed-ttl-1h")
    );
    let t = tokens_of("f4", &[("planner", both)], &criteria("f4c", SONNET_ONLY)).expect("priced");
    assert!(near(t.usd.expect("known"), 0.0143018), "{:?}", t.usd);
}

// ---- finding 5: one unknown turn makes the run's cost unknown

#[test]
fn one_incomplete_turn_makes_the_task_cost_unknown() {
    let killed: String = fixture("completed-default-ttl")
        .lines()
        .filter(|l| l.contains("\"type\":\"assistant\""))
        .map(|l| format!("{l}\n"))
        .collect();
    let t = tokens_of(
        "f5",
        &[
            ("planner", fixture("completed-default-ttl")),
            ("coder2", killed),
        ],
        &criteria("f5c", SONNET_ONLY),
    )
    .expect("reported");
    assert_eq!(t.usd, None);
}

// ---- cache lifetime and the real split

#[test]
fn five_minute_cache_writes_are_refused_because_the_ttl_was_not_pinned() {
    let err = tokens_of(
        "ttl5",
        &[("planner", fixture("completed-ttl-5m"))],
        &criteria("ttl5c", SONNET_ONLY),
    )
    .unwrap_err();
    assert!(
        err.contains("5-minute") || err.contains("total_cost_usd"),
        "{err}"
    );
}

#[test]
fn a_one_hour_run_is_priced_to_the_harness_reported_cost() {
    for (name, want) in [
        ("completed-default-ttl", 0.0086289),
        ("completed-ttl-1h", 0.0143018),
    ] {
        let t = tokens_of(
            name,
            &[("planner", fixture(name))],
            &criteria(name, SONNET_ONLY),
        )
        .expect("priced");
        assert!(near(t.usd.expect("known"), want), "{name}: {:?}", t.usd);
    }
}

// ---- fast mode, service tier, data residency

#[test]
fn fast_mode_priority_tier_and_regional_inference_are_refused_unpriced() {
    for (what, from, to) in [
        ("fast speed", "\"speed\":\"standard\"", "\"speed\":\"fast\""),
        (
            "fast state",
            "\"fast_mode_state\":\"off\"",
            "\"fast_mode_state\":\"on\"",
        ),
        (
            "priority tier",
            "\"service_tier\":\"standard\"",
            "\"service_tier\":\"priority\"",
        ),
        (
            "regional inference",
            "\"inference_geo\":\"global\"",
            "\"inference_geo\":\"us\"",
        ),
    ] {
        let text = fixture("completed-default-ttl").replace(from, to);
        assert_ne!(
            text,
            fixture("completed-default-ttl"),
            "{what}: fixture lacks {from}"
        );
        let r = tokens_of(
            "fast",
            &[("planner", text)],
            &criteria("fastc", SONNET_ONLY),
        );
        assert!(
            r.is_err(),
            "{what} must be refused, not priced at the standard rate"
        );
    }
}

// ---- price file and reconcile

#[test]
fn a_price_file_whose_one_hour_write_is_cheaper_than_its_five_minute_write_is_refused() {
    let bad = SONNET_ONLY.replace("\"cache_creation_1h\":4.0", "\"cache_creation_1h\":2.0");
    let f = scratch("bad-order").join("c.json");
    std::fs::write(&f, bad).expect("write");
    assert!(tokens::read_criteria(&f).is_err());
}

#[test]
fn reconcile_rejects_a_non_finite_api_cost() {
    let u = tokens::Usage::default();
    assert!(tokens::reconcile(&u, &u, Some((1.0, f64::NAN)), 0, 0.02).is_err());
    assert!(tokens::reconcile(&u, &u, Some((f64::NAN, 1.0)), 0, 0.02).is_err());
    assert!(tokens::reconcile(&u, &u, Some((1.0, f64::INFINITY)), 0, 0.02).is_err());
}
