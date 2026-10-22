#![allow(clippy::expect_used, clippy::unwrap_used)]
//! Stage 3a confinement: a harness launched with a `sandbox` cannot reach
//! the task directory (not `master`, not the log, not `bbp human`), yet
//! plans to the gate through its MCP relay. The same probe run unconfined is
//! the control: it reads `master`, so the probe can tell the difference.
//!
//! The confined half needs an enforcing Linux kernel (Landlock and
//! seccomp), so it runs in the required acceptance job.
#![cfg(target_os = "linux")]

mod common;
use common::Who;
use rusty_bbp::Sha256;
use rusty_bbp::*;
use rusty_bbp_host::confine::HarnessSandbox;
use rusty_bbp_host::moderator::{self, Config, Launcher, Launchers};
use rusty_bbp_host::profiles::ProfileSet;
use rusty_bbp_host::runner::Confinement;
use rusty_bbp_host::{admin, human};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Probes the task directory, prints what it could do, then plans to the
/// gate over `bbp mcp`.
const PROBE: &str = r##"#!/bin/sh
d="$1"
if cat "$d/master" >/dev/null 2>&1; then echo "master: read"; else echo "master: denied"; fi
if ls "$d" >/dev/null 2>&1; then echo "dir: listed"; else echo "dir: denied"; fi
if "$BBP_BIN" card --dir "$d" --task "$BBP_TASK" >/dev/null 2>&1; then echo "log: read"; else echo "log: denied"; fi
if echo forged >> "$d/forged" 2>/dev/null; then echo "write: done"; else echo "write: denied"; fi
"$BBP_BIN" mcp <<'EOF'
{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"sh","version":"0"}}}
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"put_artifact","arguments":{"op":"p1","kind":"spec","brief":1,"body":"# Spec\nAC1 flag says ok."}}}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"post","arguments":{"op":"p2","kind":"request_decision","to":["human"],"refs":["art:2"],"body":"Approve the plan?","gate":true}}}
EOF
"##;

fn root(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("bbp_confine_{}_{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("mkdir");
    root
}

/// Task `name` in `root/<name>`, whose planner is `script` run with `args`,
/// confined by `sandbox` or not. Every task shares `root/repo` and
/// `root/work`.
fn setup(
    root: &Path,
    name: &str,
    script: &str,
    args: Vec<String>,
    sandbox: Option<HarnessSandbox>,
) -> Config {
    let dir = root.join(name);
    let task = TaskId(name.into());
    admin::open_task(
        &dir,
        &task,
        "local",
        b"Make the flag say ok.",
        &PrincipalId("human".into()),
        &ProfileSet::shell("test", "true"),
        common::roster(),
    )
    .expect("open");
    for role in Who::ALL {
        admin::assign(
            &dir,
            &task,
            &role.id(),
            &PrincipalId(role.name().into()),
            "v",
        )
        .expect("assign");
    }
    // The script lives outside the task directory, where a sandbox may read it.
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).expect("mkdir");
    let path = bin.join(format!("{name}.sh"));
    std::fs::write(&path, script).expect("write");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    let launcher = Launcher {
        program: path.to_string_lossy().into_owned(),
        args,
        builtin_tools: true,
        sandbox,
    };
    Config {
        dir,
        task,
        bbp: PathBuf::from(env!("CARGO_BIN_EXE_bbp")),
        repo: root.join("repo"),
        work: root.join("work"),
        launchers: Launchers::default().with("planner", launcher),
        confinement: Confinement::Unconfined,
        poll: Duration::from_millis(50),
        max_wall: Duration::from_secs(60),
        end_grace: Duration::from_secs(5),
        cost_cap: None,
        slots: None,
    }
}

/// The probe as task `T1`.
fn probe(root: &Path, sandbox: Option<HarnessSandbox>) -> Config {
    setup(root, "T1", PROBE, vec!["{dir}".into()], sandbox)
}

/// Read access to the scripts, and nothing else beyond the defaults.
fn scripts(root: &Path) -> HarnessSandbox {
    HarnessSandbox {
        read: vec![root.join("bin").to_string_lossy().into_owned()],
        ..HarnessSandbox::default()
    }
}

/// Run `cfg` until the plan gate, cancel, and return what the planner said.
fn plan(cfg: Config) -> String {
    let (dir, task) = (cfg.dir.clone(), cfg.task.clone());
    let moderator = std::thread::spawn(move || moderator::run(&cfg));
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let card = admin::card(&dir, &task).expect("card");
        if card.state == State::PlanGate {
            human::perform(&dir, &task, "cancel", &["done"], Some(card.rev)).expect("cancel");
            break;
        }
        assert!(
            !moderator.is_finished() && Instant::now() < deadline,
            "no plan gate; planner said:\n{}",
            planner_log(&dir, &task)
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    let out = moderator.join().expect("join").expect("moderator");
    assert_eq!(out.state, State::Cancelled);
    planner_log(&dir, &task)
}

fn planner_log(dir: &Path, task: &TaskId) -> String {
    std::fs::read_to_string(moderator::log_path(dir, task, TurnId(1))).unwrap_or_default()
}

#[test]
fn an_unconfined_harness_reaches_the_task_directory() {
    let root = root("open");
    let said = plan(probe(&root, None));
    for line in ["master: read", "dir: listed", "log: read", "write: done"] {
        assert!(said.contains(line), "{line}:\n{said}");
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
#[ignore = "requires enforced Linux sandbox; required acceptance job"]
fn a_confined_harness_cannot_reach_the_task_directory_yet_plans_through_its_relay() {
    let root = root("confined");
    let said = plan(probe(&root, Some(scripts(&root))));
    println!("{said}");
    for line in [
        "master: denied",
        "dir: denied",
        "log: denied",
        "write: denied",
    ] {
        assert!(said.contains(line), "{line}:\n{said}");
    }
    assert!(!root.join("T1/forged").exists());
    let _ = std::fs::remove_dir_all(&root);
    println!("BBP_ACCEPTANCE PASS bbp-confined");
}

#[test]
fn a_sandbox_that_would_expose_the_task_directory_is_refused() {
    let root = root("exposed");
    // Granting the root grants the task directory under it.
    let sandbox = HarnessSandbox {
        read: vec![root.to_string_lossy().into_owned()],
        ..HarnessSandbox::default()
    };
    let cfg = probe(&root, Some(sandbox));
    let error = moderator::run(&cfg).expect_err("the moderator refuses to launch");
    assert!(error.contains("would expose the task directory"), "{error}");
    assert!(planner_log(&cfg.dir, &cfg.task).is_empty());
    let _ = std::fs::remove_dir_all(&root);
}

/// Waits for both tasks' worktrees, reports the object store it uses and
/// the checkout it sees, tries to write each worktree and the object store,
/// then plans.
const NEIGHBOUR: &str = r##"#!/bin/sh
repo="$1"; shift
for w in "$@"; do while [ ! -d "$w" ]; do sleep 0.05; done; done
echo "common: $(git rev-parse --path-format=absolute --git-common-dir)"
echo "flag: $(cat flag.txt)"
for w in "$@"; do
  if echo x > "$w/from-$BBP_TASK" 2>/dev/null; then echo "wrote: $w"; else echo "denied: $w"; fi
done
if echo x > "$repo/.git/objects/forged" 2>/dev/null; then echo "objects: wrote"; else echo "objects: denied"; fi
"$BBP_BIN" mcp >/dev/null <<'RPC'
{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"sh","version":"0"}}}
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"put_artifact","arguments":{"op":"p1","kind":"spec","brief":1,"body":"# Spec\nAC1 flag says ok."}}}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"post","arguments":{"op":"p2","kind":"request_decision","to":["human"],"refs":["art:2"],"body":"Approve the plan?","gate":true}}}
RPC
"##;

fn git(repo: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("git");
    assert!(out.status.success(), "git {args:?}");
}

#[test]
#[ignore = "requires enforced Linux sandbox; required acceptance job"]
fn two_confined_tasks_share_one_object_store_but_not_their_worktrees() {
    let root = root("worktrees");
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).expect("mkdir");
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("flag.txt"), "no\n").expect("write");
    git(&repo, &["add", "."]);
    let who = ["-c", "user.name=t", "-c", "user.email=t@example.invalid"];
    git(&repo, &[&who[..], &["commit", "-qm", "base"]].concat());
    let root = root.canonicalize().expect("canonical");
    let worktree = |task: &str| {
        root.join("work/turns")
            .join(Sha256::of(task.as_bytes()).hex())
            .join("turn-1")
    };
    let (one, two) = (worktree("T1"), worktree("T2"));
    let args: Vec<String> = [root.join("repo"), one.clone(), two.clone()]
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    let sandbox = HarnessSandbox {
        checkout: true,
        ..scripts(&root)
    };
    let runs: Vec<_> = ["T1", "T2"]
        .into_iter()
        .map(|name| {
            let cfg = setup(&root, name, NEIGHBOUR, args.clone(), Some(sandbox.clone()));
            std::thread::spawn(move || plan(cfg))
        })
        .collect();
    let said: Vec<String> = runs.into_iter().map(|r| r.join().expect("join")).collect();
    let store = root.join("repo/.git");
    for (said, (own, other)) in said.iter().zip([(&one, &two), (&two, &one)]) {
        println!("{said}");
        for line in [
            format!("common: {}", store.display()),
            "flag: no".into(),
            format!("wrote: {}", own.display()),
            format!("denied: {}", other.display()),
            "objects: denied".into(),
        ] {
            assert!(said.contains(&line), "{line}:\n{said}");
        }
    }
    assert!(!store.join("objects/forged").exists());
    // Each turn's worktree is removed when its harness stops.
    assert!(!one.exists() && !two.exists());
    let _ = std::fs::remove_dir_all(&root);
    println!("BBP_ACCEPTANCE PASS shared-worktrees");
}

/// A Codex provider stand-in: reports what it can reach from the author's
/// clone, then plans over the `bbp mcp` relay its launcher configured.
const AUTHOR: &str = r##"#!/bin/sh
cfg=$(for a in "$@"; do case "$a" in mcp_servers=*) printf '%s' "$a";; esac; done)
relay=$(printf '%s' "$cfg" | sed -n 's/.*"BBP_MCP_RELAY"="\([^"]*\)".*/\1/p')
bbp=$(printf '%s' "$cfg" | sed -n 's/^mcp_servers={bbp={command="\([^"]*\)".*/\1/p')
if cat "$TASK_DIR/master" >/dev/null 2>&1; then echo "master: read"; else echo "master: denied"; fi
echo "flag: $(cat flag.txt)"
if git add -N . && git diff --quiet; then echo "clone: usable"; else echo "clone: broken"; fi
if echo x > "$REPO/.git/objects/forged" 2>/dev/null; then echo "objects: wrote"; else echo "objects: denied"; fi
echo "alternates: $(cat "$(git rev-parse --git-dir)/objects/info/alternates")"
if env | grep -q '^BBP_'; then echo "env: bbp"; else echo "env: clean"; fi
BBP_MCP_RELAY="$relay" "$bbp" mcp >/dev/null <<'RPC'
{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"sh","version":"0"}}}
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"put_artifact","arguments":{"op":"t1-p1","kind":"spec","brief":1,"body":"# Spec\nAC1 flag says ok."}}}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"post","arguments":{"op":"t1-p2","kind":"request_decision","to":["human"],"refs":["art:2"],"body":"Approve the plan?","gate":true}}}
RPC
"##;

#[test]
#[ignore = "requires enforced Linux sandbox; required acceptance job"]
fn a_confined_codex_launcher_checks_its_turn_and_plans_from_a_shared_clone() {
    let root = root("launcher").canonicalize().expect("canonical");
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).expect("mkdir");
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("flag.txt"), "no\n").expect("write");
    git(&repo, &["add", "."]);
    let who = ["-c", "user.name=t", "-c", "user.email=t@example.invalid"];
    git(&repo, &[&who[..], &["commit", "-qm", "base"]].concat());
    let (dir, task) = (root.join("T1"), TaskId("T1".into()));
    let example: rusty_serde::json::Value =
        rusty_serde::json::from_str(include_str!("../../rusty_orch/docs/examples/bbp.json"))
            .expect("example");
    let roster: Vec<Role> = rusty_serde::json::from_str(
        &rusty_serde::json::to_string(&example["roster"]).expect("roster"),
    )
    .expect("roster");
    admin::open_task(
        &dir,
        &task,
        &repo.to_string_lossy(),
        b"Make the flag say ok.",
        &PrincipalId("human".into()),
        &ProfileSet::shell("test", "true"),
        roster,
    )
    .expect("open");
    for (role, vendor) in [("maker", "codex"), ("auditor", "claude")] {
        admin::assign(
            &dir,
            &task,
            &RoleId(role.into()),
            &PrincipalId(role.into()),
            vendor,
        )
        .expect("assign");
    }
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).expect("mkdir");
    let stub = bin.join("codex.sh");
    let script = AUTHOR.replacen(
        "#!/bin/sh\n",
        &format!(
            "#!/bin/sh\nTASK_DIR='{}'\nREPO='{}'\n",
            dir.display(),
            repo.display()
        ),
        1,
    );
    std::fs::write(&stub, script).expect("write");
    std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    let launcher = Launcher {
        program: env!("CARGO_BIN_EXE_codex-bbp-harness").into(),
        args: vec![
            "--provider-bin".into(),
            stub.to_string_lossy().into_owned(),
            "--mcp-config".into(),
            "{mcp_config}".into(),
        ],
        builtin_tools: true,
        sandbox: Some(HarnessSandbox {
            read: vec![
                bin.to_string_lossy().into_owned(),
                repo.join(".git").to_string_lossy().into_owned(),
            ],
            ..HarnessSandbox::default()
        }),
    };
    let cfg = Config {
        dir,
        task,
        bbp: PathBuf::from(env!("CARGO_BIN_EXE_bbp")),
        repo: repo.clone(),
        work: root.join("work"),
        launchers: Launchers::default().with("maker", launcher),
        confinement: Confinement::Unconfined,
        poll: Duration::from_millis(50),
        max_wall: Duration::from_secs(60),
        end_grace: Duration::from_secs(5),
        cost_cap: None,
        slots: None,
    };
    let said = plan(cfg);
    println!("{said}");
    for line in [
        "master: denied".to_owned(),
        "flag: no".into(),
        "clone: usable".into(),
        "objects: denied".into(),
        format!("alternates: {}", repo.join(".git/objects").display()),
        "env: clean".into(),
    ] {
        assert!(said.contains(&line), "{line}:\n{said}");
    }
    assert!(!repo.join(".git/objects/forged").exists());
    let _ = std::fs::remove_dir_all(&root);
    println!("BBP_ACCEPTANCE PASS launcher-confined");
}
