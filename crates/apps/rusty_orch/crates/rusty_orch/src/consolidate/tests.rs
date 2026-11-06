use super::*;
use rusty_bbp::{
    Budget, MsgId, PrincipalId, Rev, SchedulingPolicy, Sha256, TaskId, LOG_SCHEMA_CLAIMS,
};
use std::cell::RefCell;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);
fn temp() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "consolidate-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn argv(s: &str) -> Vec<String> {
    s.split_whitespace().map(String::from).collect()
}

fn log(id: &str, repo: &str, accepted: &[usize], len: usize) -> TaskLog {
    TaskLog {
        id: id.into(),
        repo: repo.into(),
        claims: true,
        open: true,
        len,
        accepted: accepted.to_vec(),
    }
}

fn journal() -> Journal {
    Journal {
        repo: "r".into(),
        dir: "/bbp".into(),
        ..Journal::default()
    }
}

#[test]
fn plan_is_the_default_and_execute_needs_its_inputs() {
    let plan = options(argv("--dir /b --repo r --repo-path /p")).unwrap();
    assert_eq!(plan.execute, None);
    assert_eq!(plan.erosion_bin, "erosion");
    assert_eq!(plan.bbp_bin, "bbp");
    let read_only = options(argv("--dir /b --repo r --repo-path /p --bbp-bin /x/bbp")).unwrap();
    assert_eq!(
        (read_only.bbp_bin.as_str(), read_only.execute),
        ("/x/bbp", None)
    );
    for bad in [
        "--dir /b --repo r",
        "--dir /b --repo r --repo-path /p --roster x",
        "--dir /b --repo r --repo-path /p --execute --profiles p --roster x",
        "--dir /b --repo r --repo-path /p --execute --profiles p --roster x --work w --holdout-root h",
        "--dir /b --dir /c --repo r --repo-path /p",
        "--dir /b --repo r --repo-path /p --bogus 1",
    ] {
        assert!(matches!(options(argv(bad)), Err(ArgsError::Usage(_))), "{bad}");
    }
    let e = options(argv(
        "--dir /b --repo r --repo-path /p --execute --profiles p --roster x --work w --claims c",
    ))
    .unwrap()
    .execute
    .unwrap();
    assert_eq!(
        e.passthrough,
        [
            ("--roster", "x".into()),
            ("--work", "w".into()),
            ("--claims", "c".into())
        ]
    );
}

#[test]
fn five_accepted_claims_across_tasks_since_the_marker_make_a_cleanup_due() {
    let mut j = journal();
    let tasks = [log("a", "r", &[1, 2, 3], 9), log("b", "r", &[4], 5)];
    assert_eq!(due(&tasks, &j), Due::Wait(4));
    let tasks = [log("a", "r", &[1, 2, 3], 9), log("b", "r", &[4, 6], 7)];
    let Due::Now { accepted, snapshot } = due(&tasks, &j) else {
        panic!()
    };
    assert_eq!(accepted, 5);
    assert_eq!(snapshot, BTreeMap::from([("a".into(), 9), ("b".into(), 7)]));
    // After the marker only later acceptances count.
    j.marker = snapshot;
    let tasks = [
        log("a", "r", &[1, 2, 3, 9, 10], 11),
        log("b", "r", &[4, 6], 7),
    ];
    assert_eq!(due(&tasks, &j), Due::Wait(2));
}

#[test]
fn other_repos_stage_tasks_and_cleanups_do_not_count() {
    let mut j = journal();
    j.cleanups.push("c".into());
    let mut stages = log("s", "r", &[1, 2, 3, 4, 5], 6);
    stages.claims = false;
    let mut cleanup = log("c", "r", &[1, 2, 3, 4, 5], 6);
    cleanup.open = false;
    let tasks = [log("o", "other", &[1, 2, 3, 4, 5], 6), stages, cleanup];
    assert_eq!(due(&tasks, &j), Due::Wait(0));
}

#[test]
fn an_open_cleanup_blocks_a_new_one() {
    let mut j = journal();
    j.cleanups.push("c".into());
    let tasks = [log("c", "r", &[], 1), log("a", "r", &[1, 2, 3, 4, 5], 6)];
    assert_eq!(due(&tasks, &j), Due::Busy("c".into()));
}

#[test]
fn a_pending_open_settles_by_whether_the_store_has_the_task() {
    let mut j = journal();
    let snapshot = BTreeMap::from([("a".to_string(), 6)]);
    j.pending = Some(Pending {
        task: "c".into(),
        base: base("b", 10, 20),
        snapshot: snapshot.clone(),
    });
    let missing = j.clone().settle(&[log("a", "r", &[], 6)]);
    assert_eq!(missing, journal());
    let landed = j.settle(&[log("c", "r", &[], 1)]);
    assert_eq!(landed.marker, snapshot);
    assert_eq!(landed.cleanups, ["c"]);
    assert_eq!(landed.pending, None);
    assert_eq!(landed.last_base, Some(base("b", 10, 20)));
}

fn base(commit: &str, c: i64, d: i64) -> Base {
    Base {
        commit: commit.into(),
        concentration_bp: c,
        duplication_bp: d,
    }
}

fn levels(c: i64, d: i64) -> Levels {
    Levels {
        concentration_bp: c,
        duplication_bp: d,
    }
}

#[test]
fn the_half_leg_fires_on_a_rise_of_half_either_limit() {
    let lim = |c, d| {
        Some(Limits {
            concentration_bp: c,
            duplication_bp: d,
        })
    };
    let last = base("b", 100, 50);
    assert_eq!(half(None, Some(&last), levels(999, 999)), Half::Inactive);
    assert_eq!(half(lim(10, 10), None, levels(999, 999)), Half::Start);
    assert_eq!(half(lim(10, 10), Some(&last), levels(104, 54)), Half::Below);
    assert_eq!(
        half(lim(10, 10), Some(&last), levels(105, 50)),
        Half::Reached
    );
    assert_eq!(
        half(lim(10, 10), Some(&last), levels(90, 55)),
        Half::Reached
    );
    assert_eq!(half(lim(1, 1), Some(&last), levels(101, 50)), Half::Reached);
    // D120: no rise never fires, even against a 0 bp limit.
    assert_eq!(half(lim(0, 0), Some(&last), levels(100, 49)), Half::Below);
    assert_eq!(half(lim(0, 0), Some(&last), levels(100, 51)), Half::Reached);
}

#[test]
fn limits_parse_the_newest_record_or_null() {
    assert_eq!(Limits::parse("null\n").unwrap(), None);
    let r = r#"{"at_ms":1,"concentration_bp":1,"duplication_bp":2,"by":"nano","source":"cmsg"}"#;
    assert_eq!(
        Limits::parse(r).unwrap(),
        Some(Limits {
            concentration_bp: 1,
            duplication_bp: 2
        })
    );
    assert!(Limits::parse("{}").is_err());
    assert!(Limits::parse("nope").is_err());
}

#[test]
fn levels_come_from_the_base_shares_and_errors_are_refused() {
    let line = r#"{"erosion":1,"base":"x","mode":"report-only","within_limits":null,"concentration_bp":{"base":2380,"candidate":2381,"delta":1},"duplication_bp":{"base":0,"candidate":0,"delta":0}}"#;
    let l = Levels::parse(line).unwrap();
    assert_eq!((l.concentration_bp, l.duplication_bp), (2380, 0));
    assert!(!l.zero());
    assert!(Levels {
        concentration_bp: 0,
        duplication_bp: 0
    }
    .zero());
    assert!(Levels::parse(r#"{"erosion":1,"error":"bad"}"#)
        .unwrap_err()
        .contains("bad"));
    assert!(Levels::parse(r#"{"erosion":1}"#).is_err());
}

const BASE: &str = "0123456789abcdef0123456789abcdef01234567";

fn set(args: &str) -> String {
    let args: Vec<String> = args.split_whitespace().map(|a| format!("{a:?}")).collect();
    format!(
        r#"{{"profiles":[{{"name":"test","program":"/bin/sh","args":["-c","true"]}},{{"name":"erosion","program":"/usr/local/bin/erosion","args":[{}]}}],"read_roots":[]}}"#,
        args.join(",")
    )
}

#[test]
fn the_cleanup_profile_gets_the_measured_base_and_other_modes_are_refused() {
    let out = cleanup_profiles(&set("check --base HEAD --cleanup"), BASE).unwrap();
    assert!(
        out.contains(&format!(r#"["check","--base","{BASE}","--cleanup"]"#)),
        "{out}"
    );
    assert!(out.contains(r#""read_roots":[]"#), "{out}");
    for bad in [
        set("check --base HEAD --report-only"),
        set("check --base HEAD --max-concentration-bp 1 --max-duplication-bp 1"),
        set("check --cleanup --base"),
        r#"{"profiles":[]}"#.into(),
        "nope".into(),
    ] {
        assert!(cleanup_profiles(&bad, BASE).is_err(), "{bad}");
    }
}

/// A claims task in a real store, with `accepted` acceptances.
fn persist(dir: &Path, name: &str, repo: &str, accepted: u64) {
    let task = TaskId(name.into());
    let mut events = vec![Event::TaskOpened {
        claims: None,
        mode: Mode::Claims,
        schema: LOG_SCHEMA_CLAIMS,
        roster: vec![],
        task: task.clone(),
        repo: repo.into(),
        profile_digest: Sha256::of(b"profiles"),
        budget: Budget {
            messages: 10,
            bytes: 1000,
            reads: 10,
            reads_per_turn: 10,
            turns: 10,
            iterations: 10,
        },
        turn_ms: 1000,
        request_ms: 1000,
        max_live_turns: 1,
        human: PrincipalId("human".into()),
        policy: SchedulingPolicy::default(),
        holdout_digest: None,
        baseline_digest: None,
    }];
    events.extend((0..accepted).map(|i| Event::ClaimAccepted {
        claim: MsgId(i),
        score: 1,
        evidence: 1,
        sole_candidate: false,
        supersedes_lapsed: None,
    }));
    FsStore::open(dir)
        .unwrap()
        .append(&task, Rev(0), &events)
        .unwrap();
}

#[test]
fn a_store_log_reads_as_its_repo_mode_and_acceptances() {
    let dir = temp();
    persist(&dir, "t", "r", 2);
    let tasks = read_tasks(&dir).unwrap();
    assert_eq!(
        tasks,
        [TaskLog {
            id: "t".into(),
            repo: "r".into(),
            claims: true,
            open: true,
            len: 3,
            accepted: vec![1, 2]
        }]
    );
    assert!(read_tasks(&dir.join("none")).unwrap().is_empty());
    fs::remove_dir_all(dir).unwrap();
}

/// Answers git and erosion; `bbp open` writes the task it was asked for.
struct Fake {
    common: PathBuf,
    levels: RefCell<(i64, i64)>,
    limits: Option<(i64, i64)>,
    calls: RefCell<Vec<Vec<String>>>,
}

impl CommandRunner for Fake {
    fn run_in(
        &self,
        _cwd: Option<&Path>,
        argv: &[String],
        _stdin: &[u8],
        _timeout: Duration,
        _remove_env: &[&str],
    ) -> Result<orch_cli::Exit, orch_cli::ExecError> {
        self.calls.borrow_mut().push(argv.to_vec());
        let flag = |f: &str| {
            argv.iter()
                .position(|a| a == f)
                .map(|i| argv[i + 1].clone())
        };
        let stdout = match (argv[0].as_str(), argv[1].as_str()) {
            ("git", "rev-parse") if argv.contains(&"--git-common-dir".to_string()) => {
                self.common.display().to_string()
            }
            ("git", "rev-parse") => BASE.into(),
            ("erosion", "check") => format!(
                r#"{{"erosion":1,"concentration_bp":{{"base":{}}},"duplication_bp":{{"base":{}}}}}"#,
                self.levels.borrow().0,
                self.levels.borrow().1
            ),
            ("bbp", "erosion") => match self.limits {
                None => "null".into(),
                Some((c, d)) => format!(r#"{{"concentration_bp":{c},"duplication_bp":{d}}}"#),
            },
            ("bbp", "open") => {
                let dir = PathBuf::from(flag("--dir").unwrap());
                persist(&dir, &flag("--task").unwrap(), &flag("--repo").unwrap(), 0);
                "{}".into()
            }
            _ => panic!("unexpected {argv:?}"),
        };
        Ok(orch_cli::Exit {
            status: 0,
            stdout: stdout.into_bytes(),
            stderr: vec![],
        })
    }
}

fn setup(levels: (i64, i64)) -> (PathBuf, Options, Fake) {
    setup_with(levels, None)
}

fn setup_with(levels: (i64, i64), limits: Option<(i64, i64)>) -> (PathBuf, Options, Fake) {
    let root = temp();
    let profiles = root.join("profiles.json");
    fs::write(&profiles, set("check --base HEAD --cleanup")).unwrap();
    let options = Options {
        dir: root.join("bbp"),
        repo: "r".into(),
        repo_path: root.join("repo"),
        erosion_bin: "erosion".into(),
        bbp_bin: "bbp".into(),
        execute: Some(Execute {
            profiles,
            passthrough: vec![("--roster", "x".into()), ("--work", "w".into())],
        }),
    };
    let fake = Fake {
        common: root.join("git"),
        levels: RefCell::new(levels),
        limits,
        calls: RefCell::default(),
    };
    (root, options, fake)
}

#[test]
fn execute_opens_a_cleanup_then_waits_for_it_and_for_five_new_claims() {
    let (root, options, fake) = setup((2380, 3043));
    persist(&options.dir, "a", "r", 3);
    persist(&options.dir, "b", "r", 2);
    let plan = Options {
        execute: None,
        ..options.clone()
    };
    let out = run_with(&plan, &fake, 7).unwrap();
    assert!(out.contains(r#""action":"open""#), "{out}");
    assert!(
        !root.join("git").join(STATE).exists(),
        "a plan writes nothing"
    );

    let out = run_with(&options, &fake, 7).unwrap();
    let task = format!("consolidate-{}-7", &BASE[..12]);
    assert!(
        out.contains(r#""action":"opened""#) && out.contains(&task),
        "{out}"
    );
    let open = fake
        .calls
        .borrow()
        .iter()
        .find(|c| c[..2] == ["bbp", "open"])
        .cloned()
        .unwrap();
    for pair in [
        ["--mode", "claims"],
        ["--repo", "r"],
        ["--base", BASE],
        ["--roster", "x"],
    ] {
        assert!(open.windows(2).any(|w| w == pair), "{open:?}");
    }
    let frozen = fs::read_to_string(
        root.join("git")
            .join(STATE)
            .join(&task)
            .join("profiles.json"),
    )
    .unwrap();
    assert!(frozen.contains(BASE), "{frozen}");

    let out = run_with(&options, &fake, 8).unwrap();
    assert!(out.contains(r#""action":"busy""#), "{out}");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_zero_floor_skips_and_moves_the_marker() {
    let (root, options, fake) = setup((0, 0));
    persist(&options.dir, "a", "r", 5);
    let out = run_with(&options, &fake, 7).unwrap();
    assert!(out.contains(r#""action":"skipped""#), "{out}");
    assert!(!fake.calls.borrow().iter().any(|c| c[1] == "open"));
    let out = run_with(&options, &fake, 8).unwrap();
    assert!(
        out.contains(r#""action":"wait""#) && out.contains(r#""accepted":0"#),
        "{out}"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_journal_for_another_repo_is_refused() {
    let (root, options, fake) = setup((0, 0));
    persist(&options.dir, "a", "r", 5);
    run_with(&options, &fake, 7).unwrap();
    let other = Options {
        repo: "other".into(),
        ..options
    };
    assert!(run_with(&other, &fake, 8)
        .unwrap_err()
        .contains("belongs to repo"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_terminal_cleanup_no_longer_blocks() {
    let mut j = journal();
    j.cleanups.push("c".into());
    let mut done = log("c", "r", &[], 3);
    done.open = false;
    assert_eq!(due(&[done], &j), Due::Wait(0));
}

#[test]
fn without_limits_a_waiting_run_measures_nothing() {
    let (root, options, fake) = setup((10, 10));
    persist(&options.dir, "a", "r", 1);
    let out = run_with(&options, &fake, 7).unwrap();
    assert!(out.contains(r#""half":"inactive""#), "{out}");
    assert!(!fake.calls.borrow().iter().any(|c| c[0] == "erosion"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn the_half_leg_starts_at_head_then_opens_on_half_a_limit() {
    let (root, options, fake) = setup_with((100, 50), Some((10, 10)));
    persist(&options.dir, "a", "r", 1);
    let plan = Options {
        execute: None,
        ..options.clone()
    };
    let out = run_with(&plan, &fake, 6).unwrap();
    assert!(out.contains(r#""half":"start""#), "{out}");
    assert!(
        !root.join("git").join(STATE).exists(),
        "a plan writes nothing"
    );

    let out = run_with(&options, &fake, 7).unwrap();
    assert!(
        out.contains(r#""half":"start""#) && out.contains(r#""action":"wait""#),
        "{out}"
    );
    *fake.levels.borrow_mut() = (104, 50);
    let out = run_with(&options, &fake, 8).unwrap();
    assert!(
        out.contains(r#""half":"below""#) && out.contains(r#""last_base""#),
        "{out}"
    );
    *fake.levels.borrow_mut() = (105, 50);
    let out = run_with(&options, &fake, 9).unwrap();
    assert!(
        out.contains(r#""action":"opened""#) && out.contains(r#""trigger":"half""#),
        "{out}"
    );
    // The open moved the base to the levels it measured, and the marker.
    let journal: Journal = json::from_str(
        &fs::read_to_string(root.join("git").join(STATE).join("journal.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(journal.last_base, Some(base(BASE, 105, 50)));
    assert_eq!(journal.marker, BTreeMap::from([("a".into(), 2)]));
    fs::remove_dir_all(root).unwrap();
}
