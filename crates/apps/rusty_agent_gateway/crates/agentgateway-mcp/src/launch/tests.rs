use std::collections::BTreeMap;

use super::*;

fn launch(cmd: &str, args: &[&str], env: &[(&str, &str)]) -> Launch {
    Launch::new(
        &StdioLaunch {
            cmd: cmd.to_owned(),
            args: args.iter().map(|a| (*a).to_owned()).collect(),
            env: env
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect::<BTreeMap<_, _>>(),
        },
        "t.stdio.session",
    )
    .expect("launch compiles")
}

fn session() -> ClassicSession {
    ClassicSession::new("s1")
}

fn claims(claim: &str) -> serde_json::Value {
    serde_json::json!({ "sub": "s", "lel.claim": claim, "lel.lens": "builder-rust" })
}

#[test]
fn claims_fill_cmd_args_and_env() {
    let l = launch(
        "lel-jail",
        &[
            "/srv/lel/{{jwt.lel.claim}}/jail.json",
            "--mode",
            "{{jwt.lel.lens:7}}",
        ],
        &[("LEL_CLAIM", "{{jwt.lel.claim}}")],
    );
    let r = l
        .resolve(Some(&session()), Some(&claims("c-0123")))
        .expect("resolves");
    assert_eq!(r.cmd, "lel-jail");
    assert_eq!(r.args, ["/srv/lel/c-0123/jail.json", "--mode", "builder"]);
    assert_eq!(r.env, [("LEL_CLAIM".to_owned(), "c-0123".to_owned())]);
}

#[test]
fn a_literal_launch_still_needs_a_session_and_a_token() {
    let l = launch("orch-fs-shell", &["--mode", "critic"], &[]);
    let token = serde_json::json!({"sub": "s"});
    for (session, claims) in [(None, None), (None, Some(&token)), (Some(session()), None)] {
        let err = l.resolve(session.as_ref(), claims).expect_err("unverified");
        assert!(matches!(err, IdentityError::Unverified), "{err}");
    }
    let r = l
        .resolve(Some(&session()), Some(&token))
        .expect("nothing to fill");
    assert_eq!(r.args, ["--mode", "critic"]);
}

#[test]
fn values_that_could_leave_a_segment_are_refused() {
    let l = launch("x", &["/srv/{{jwt.lel.claim}}/jail.json"], &[]);
    for bad in ["..", ".", "a/b", "../x", "a b", "a=b", "caf\u{e9}", "x\0"] {
        let err = l
            .resolve(Some(&session()), Some(&claims(bad)))
            .expect_err(bad);
        assert_eq!(
            err,
            IdentityError::Value {
                claim: "lel.claim".to_owned()
            },
            "{bad:?}"
        );
    }
}

#[test]
fn a_cut_that_leaves_dots_is_refused() {
    let l = launch("x", &["{{jwt.lel.claim:2}}"], &[]);
    assert!(l.resolve(Some(&session()), Some(&claims("..ok"))).is_err());
    assert_eq!(
        l.resolve(Some(&session()), Some(&claims("ab-c")))
            .expect("cut")
            .args,
        ["ab"]
    );
}

#[test]
fn a_missing_claim_refuses() {
    let err = launch("x", &["{{jwt.lel.claim}}"], &[])
        .resolve(Some(&session()), Some(&serde_json::json!({"sub": "s"})))
        .expect_err("missing");
    assert_eq!(
        err,
        IdentityError::Unresolved {
            claim: "lel.claim".to_owned()
        }
    );
}

#[test]
fn bad_templates_are_refused_at_startup() {
    let err = Launch::new(
        &StdioLaunch {
            cmd: "x".to_owned(),
            args: vec!["{{jwt.lel.claim".to_owned()],
            env: BTreeMap::new(),
        },
        "t.stdio.session",
    )
    .expect_err("unclosed");
    assert!(err.to_string().contains("t.stdio.session.args[0]"), "{err}");
}

#[test]
fn keys_differ_when_any_part_differs() {
    let l = launch("x", &["{{jwt.lel.claim}}"], &[]);
    let a = l
        .resolve(Some(&session()), Some(&claims("a")))
        .expect("a")
        .key();
    let b = l
        .resolve(Some(&session()), Some(&claims("b")))
        .expect("b")
        .key();
    assert_ne!(a, b);
}
