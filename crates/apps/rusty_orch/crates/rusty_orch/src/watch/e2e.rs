//! R15 replay through a real `bbp` host: the watcher's posts on a claims
//! task, with no model calls. Run by the Orch BBP acceptance job.
use super::tests::{set, MemSeen};
use super::*;

use std::process::Command;

fn bbp(bin: &Path, args: &[&str]) {
    let out = Command::new(bin).args(args).output().expect("run bbp");
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && !said.contains("Rejected"),
        "bbp {args:?}: {said} {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn git(repo: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@example.invalid"])
        .args(args)
        .current_dir(repo)
        .status()
        .expect("run git")
        .success();
    assert!(ok, "git {args:?}");
}

/// A claims task `maint-demo` with `watcher` and one other role assigned.
fn open_task(root: &Path, bin: &Path) -> PathBuf {
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(repo.join("f"), "ok\n").unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["add", "f"]);
    git(&repo, &["commit", "-qm", "base"]);
    let shipped = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../rusty_bbp_host/docs/proving/profiles.json"
    );
    let mut profiles: Value =
        rusty_serde::json::from_str(&std::fs::read_to_string(shipped).unwrap()).unwrap();
    if let Value::Map(m) = &mut profiles {
        m.retain(|(k, _)| k != "test_paths" && k != "erosion");
        m.push(("test_paths".into(), Value::Seq(vec!["tests/**".into()])));
        m.push(("erosion".into(), "off".into()));
    }
    let role = |id: &str, order: u32| {
        format!(
            r#"{{"id":"{id}","turn_order":{order},"post":["Claim","Evidence"],"write":["Diff","Candidate","Source"],"read_artifacts":["Brief","Diff","Candidate","TestReport","Log","Source"],"read_messages":"All","verdict_at":[],"final_decision":false,"consultable":false}}"#
        )
    };
    let files = [
        (
            "profiles.json",
            rusty_serde::json::to_string(&profiles).unwrap(),
        ),
        (
            "roster.json",
            format!("[{},{}]", role(WATCHER, 0), role("a", 1)),
        ),
        ("brief.md", "Maintain demo.\n".to_owned()),
    ];
    for (name, body) in &files {
        std::fs::write(root.join(name), body).unwrap();
    }
    let dir = root.join("bbp");
    let p = |n: &str| root.join(n).display().to_string();
    let d = dir.display().to_string();
    let repo_s = repo.display().to_string();
    bbp(
        bin,
        &[
            "open",
            "--mode",
            "claims",
            "--dir",
            &d,
            "--task",
            "maint-demo",
            "--repo",
            &repo_s,
            "--brief",
            &p("brief.md"),
            "--roster",
            &p("roster.json"),
            "--profiles",
            &p("profiles.json"),
        ],
    );
    for r in [WATCHER, "a"] {
        let args = ["assign", "--dir", &d, "--task", "maint-demo", "--role", r];
        bbp(
            bin,
            &[&args[..], &["--principal", r, "--vendor", "test"]].concat(),
        );
    }
    dir
}

#[test]
#[ignore = "requires built bbp at BBP_TEST_BIN"]
fn findings_are_posted_once_and_superseded_through_a_real_bbp() {
    let bin = PathBuf::from(std::env::var_os("BBP_TEST_BIN").expect("BBP_TEST_BIN"))
        .canonicalize()
        .unwrap();
    let root = std::env::temp_dir().join(format!("orch_watch_e2e_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let dir = open_task(&root, &bin);
    let session = || Child::spawn(&bin, &dir, "maint-demo", WATCHER).unwrap();
    let mut child = session();
    let mut seen = MemSeen::default();

    // Changed details: the first claim stands and nothing is posted.
    let deny = |title: &str, cvss: &str| {
        advisories(&format!(
            r#"{{"type":"diagnostic","fields":{{"advisory":{{"id":"RUSTSEC-2026-0001","package":"foo","title":"{title}","cvss":"{cvss}"}}}}}}"#
        ))
        .remove(0)
    };
    let a = record(
        &mut child,
        &mut seen,
        "demo",
        deny("Use after free", "AV:N"),
    )
    .unwrap();
    let b = record(
        &mut child,
        &mut seen,
        "demo",
        deny("Use after free in parser", "AV:L"),
    )
    .unwrap();
    assert!(a.new && !b.new && a.claim == b.claim, "{a:?} {b:?}");

    // A new update set supersedes the open claim; the host admits it.
    let first = record(&mut child, &mut seen, "demo", set("v1.0.1")).unwrap();
    let next = record(&mut child, &mut seen, "demo", set("v1.0.2")).unwrap();
    assert!(next.new && next.supersedes == first.claim, "{next:?}");

    // A key that comes back (A, B, A) is a new op over B: a new claim.
    let back = record(&mut child, &mut seen, "demo", set("v1.0.1")).unwrap();
    assert!(back.new && back.supersedes == next.claim, "{back:?}");
    assert_ne!(back.claim, first.claim);

    // C posts but its settling write is lost, and the process restarts.
    seen.fail_after = Some(1);
    let lost = record(&mut child, &mut seen, "demo", set("v1.0.3")).unwrap_err();
    assert!(lost.contains("recorded next run"), "{lost}");
    drop(child);
    let mut child = session();
    // The next run sees D: C's resend replays, and D supersedes C, not A.
    let d = record(&mut child, &mut seen, "demo", set("v1.0.4")).unwrap();
    let c = d.supersedes.expect("supersedes C");
    assert_ne!(Some(c), back.claim, "C was recovered first");
    let again = post(&mut child, "demo", &set("v1.0.3"), back.claim).unwrap();
    assert_eq!(again, c, "C's post replays to the recovered claim");

    // The host really compares payloads: drifted text under the same op is
    // refused, so the replays above are not a lenient double.
    let f = set("v1.0.3");
    let drifted = Finding {
        text: format!("{} Now.", f.text),
        ..f
    };
    let e = post(&mut child, "demo", &drifted, back.claim).unwrap_err();
    assert!(e.contains("OpConflict"), "{e}");
    drop(child);
    let _ = std::fs::remove_dir_all(&root);
    println!("BBP_ACCEPTANCE PASS watch-replay");
}
