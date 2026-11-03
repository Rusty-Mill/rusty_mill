//! The `BbpCli` adapter against a real `bbp` host built from this tree: the
//! inbox rows it reads and the commands it sends must match what the host
//! accepts, which the stand-in of `inbox_cli.rs` cannot prove.
//!
//! Opt-in: `BBP_TEST_BIN` names the built `bbp`. CI's confined acceptance
//! lane sets it and runs this with `--ignored`; there a missing or broken
//! host fails the test instead of skipping it. Claims mode needs a working
//! sandbox, which that lane has.
#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use rusty_mill_board::bbp_cli::BbpCli;
use rusty_mill_board::inbox::{Action, Escalation, Escalations, Outcome, Reason, Response};

const ROSTER: &str = r#"[{"id":"a","turn_order":0,"post":["Claim","Evidence"],"write":["Diff","Candidate","Source"],"read_artifacts":["Brief","Diff","Candidate","TestReport","Log","Source"],"read_messages":"All","verdict_at":[],"final_decision":false,"consultable":false},{"id":"b","turn_order":1,"post":["Claim","Evidence"],"write":["Diff","Candidate","Source"],"read_artifacts":["Brief","Diff","Candidate","TestReport","Log","Source"],"read_messages":"All","verdict_at":[],"final_decision":false,"consultable":false}]"#;

const PROFILES: &str = r#"{"profiles":[{"name":"test","program":"/bin/sh","args":["-c","sh ./test.sh"]}],"read_roots":["/bin","/usr","/lib","/lib64"],"env":[["PATH","/usr/bin:/bin"]],"limits":{"cpu_secs":60,"wall_secs":120,"memory_bytes":1073741824,"file_bytes":268435456,"open_files":256,"processes":4096},"tree":"sha256(git write-tree id)","test_paths":["tests/**"]}"#;

/// The task opens with 40 messages; this is the one allowance Approve adds.
const ALLOWANCE: u64 = 40;

/// The host under test. Unset means a misconfigured lane: fail, never skip.
fn bbp() -> PathBuf {
    let bin = std::env::var_os("BBP_TEST_BIN").expect("set BBP_TEST_BIN to the built bbp");
    let bin = PathBuf::from(bin);
    assert!(
        bin.is_file(),
        "BBP_TEST_BIN is not a file: {}",
        bin.display()
    );
    bin
}

/// Run `bbp` and return its stdout; any failure to run or a non-zero exit panics.
fn run(bin: &Path, args: &[&str], env: &[(&str, &str)], stdin: Option<&str>) -> String {
    use std::io::Write;
    use std::process::Stdio;
    let mut cmd = Command::new(bin);
    cmd.args(args)
        .envs(env.iter().copied())
        .stdout(Stdio::piped());
    cmd.stdin(if stdin.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    let mut child = cmd.spawn().expect("spawn bbp");
    if let Some(input) = stdin {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
    }
    let out = child.wait_with_output().expect("bbp");
    assert!(out.status.success(), "bbp {args:?} exited {}", out.status);
    String::from_utf8(out.stdout).unwrap()
}

/// A claims task `task` under `root` whose message total is cut to one and
/// then crossed by two claims, so it pauses with a task-total escalation.
fn paused_task(bin: &Path, root: &Path, task: &str) {
    let files = root.join(format!("{task}-files"));
    fs::create_dir_all(&files).unwrap();
    for (name, body) in [
        ("roster.json", ROSTER),
        ("profiles.json", PROFILES),
        ("brief.md", "Make the flag say ok."),
    ] {
        fs::write(files.join(name), body).unwrap();
    }
    let dir = root.to_str().unwrap();
    let file = |n: &str| files.join(n).to_str().unwrap().to_owned();
    let (roster, profiles, brief) = (file("roster.json"), file("profiles.json"), file("brief.md"));
    let opened = run(
        bin,
        &[
            "open",
            "--mode",
            "claims",
            "--confine",
            "sandbox",
            "--dir",
            dir,
            "--task",
            task,
            "--repo",
            "local",
            "--brief",
            &brief,
            "--roster",
            &roster,
            "--profiles",
            &profiles,
        ],
        &[],
        None,
    );
    assert!(opened.contains("Stored"), "open: {opened}");
    for who in ["a", "b"] {
        let r = run(
            bin,
            &[
                "assign",
                "--dir",
                dir,
                "--task",
                task,
                "--role",
                who,
                "--principal",
                who,
                "--vendor",
                "v",
            ],
            &[],
            None,
        );
        assert!(r.contains("Ok"), "assign {who}: {r}");
    }
    let r = run(
        bin,
        &[
            "human", "--dir", dir, "--task", task, "extend", "messages", "1",
        ],
        &[],
        None,
    );
    assert!(r.contains("Ok"), "extend: {r}");
    let post = |n: u8| {
        format!(
            r#"{{"jsonrpc":"2.0","id":{n},"method":"tools/call","params":{{"name":"post","arguments":{{"op":"c{n}","kind":"claim","text":"claim {n}","domains":["d"],"stakes":"low","reversible":true,"refs":["art:1"]}}}}}}"#
        )
    };
    let session = [
        ("BBP_DIR", dir),
        ("BBP_TASK", task),
        ("BBP_PRINCIPAL", "a"),
        ("BBP_SESSION", "yes"),
    ];
    let out = run(
        bin,
        &["mcp"],
        &session,
        Some(&format!("{}\n{}\n", post(1), post(2))),
    );
    assert_eq!(out.matches(r#"\"Posted\""#).count(), 2, "posts: {out}");
}

/// `bbp card` of `task`, as text.
fn card(bin: &Path, root: &Path, task: &str) -> String {
    let dir = root.to_str().unwrap();
    run(bin, &["card", "--dir", dir, "--task", task], &[], None)
}

/// The one open escalation of `task`.
fn only_row(cli: &BbpCli, task: &str) -> Escalation {
    let mut rows = cli.open(&[task.to_owned()]).unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    rows.remove(0)
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mill_real_bbp_{}_{tag}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
#[ignore = "requires the built bbp at BBP_TEST_BIN and a working sandbox"]
fn the_inbox_drives_a_real_bbp_host() {
    let bin = bbp();
    let root = scratch("inbox");
    let cli = BbpCli::new(bin.clone(), root.clone());

    // Approve on a task total: the row names the crossed total, the preview
    // is the one-allowance extend, and the task resumes with exactly that.
    paused_task(&bin, &root, "C1");
    let row = only_row(&cli, "C1");
    assert_eq!(row.reason, Reason::TaskTotal);
    assert_eq!(row.acted, Some(vec![]), "this host reports acted-on work");
    let totals = row.totals.clone().expect("this host reports totals");
    assert_eq!(totals.len(), 1, "{totals:?}");
    assert_eq!(
        (
            totals[0].field.as_str(),
            totals[0].limit,
            totals[0].allowance
        ),
        ("Messages", 1, ALLOWANCE)
    );
    let dry = cli.preview(&row, &Action::Approve).unwrap();
    assert_eq!(
        (dry.step.verb, dry.step.args.clone()),
        (
            "extend",
            vec!["messages".into(), (1 + ALLOWANCE).to_string()]
        )
    );
    assert!(matches!(dry.response, Response::Accepted(_)), "{dry:?}");
    assert!(!dry.events.is_empty());
    assert_eq!(
        cli.act(&row, &Action::Approve, "owner").unwrap(),
        Outcome::Done
    );
    assert!(cli.open(&["C1".into()]).unwrap().is_empty());
    let after = card(&bin, &root, "C1");
    assert!(after.contains(r#""state":"Open""#), "{after}");
    assert!(
        after.contains(&format!(r#""messages":{}"#, 1 + ALLOWANCE)),
        "{after}"
    );

    // The same decision sent again is fenced out: no second allowance.
    let again = cli.act(&row, &Action::Approve, "owner").unwrap();
    assert!(
        matches!(&again, Outcome::Refused { step: 0, .. }),
        "{again:?}"
    );
    assert_eq!(card(&bin, &root, "C1"), after);

    // Refine on a task total: the human answer refs the brief, then one
    // allowance, then resume; the task leaves the inbox.
    paused_task(&bin, &root, "C2");
    let row = only_row(&cli, "C2");
    assert!(row.brief.is_some(), "{row:?}");
    let refine = Action::Refine {
        note: "smaller steps".into(),
    };
    assert_eq!(cli.act(&row, &refine, "owner").unwrap(), Outcome::Done);
    assert!(cli.open(&["C2".into()]).unwrap().is_empty());
    let after = card(&bin, &root, "C2");
    assert!(after.contains(r#""state":"Open""#), "{after}");

    let _ = fs::remove_dir_all(&root);
    println!("BBP_ACCEPTANCE PASS millboard-inbox");
}
