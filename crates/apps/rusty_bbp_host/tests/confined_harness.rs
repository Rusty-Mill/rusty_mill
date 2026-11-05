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
if kill -0 "$PPID" 2>/dev/null; then echo "signal: sent"; else echo "signal: denied"; fi
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
        session: false,
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
        deadline: None,
        cost_cap: None,
        admission: None,
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
    for line in [
        "master: read",
        "dir: listed",
        "log: read",
        "write: done",
        "signal: sent",
    ] {
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
    // Below Landlock ABI v6 a confined harness can still signal its user's
    // processes; from v6 on it cannot reach the moderator that started it.
    let signal = if rusty_sandbox::signals_scoped() {
        "signal: denied"
    } else {
        "signal: sent"
    };
    assert!(said.contains(signal), "{signal}:\n{said}");
    assert!(!root.join("T1/forged").exists());
    let _ = std::fs::remove_dir_all(&root);
    println!("BBP_ACCEPTANCE PASS bbp-confined");
}

/// A confined session planner: it reports what its turn finds, leaves a file
/// in its workspace and counts its turns in `$HOME`, passing the first three
/// and planning on the fourth. It reaches the gate only if `$HOME` outlives
/// each turn.
const SESSION: &str = r##"#!/bin/sh
set -e
echo "turn $BBP_TURN resume $BBP_SESSION_RESUME cwd $PWD left $(ls -A | wc -l)"
echo "$BBP_TURN" >> "$HOME/turns"
touch left-behind
init='{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"sh","version":"0"}}}'
call() { printf '{"jsonrpc":"2.0","id":%s,"method":"tools/call","params":%s}\n' "$1" "$2"; }
if [ "$(wc -l < "$HOME/turns")" -le 3 ]; then
  { echo "$init"; call 1 '{"name":"post","arguments":{"op":"pass-'"$BBP_TURN"'","kind":"pass"}}'; } | "$BBP_BIN" mcp
else
  { echo "$init"
    call 1 '{"name":"put_artifact","arguments":{"op":"s","kind":"spec","brief":1,"body":"# Spec"}}'
    call 2 '{"name":"post","arguments":{"op":"g","kind":"request_decision","to":["human"],"refs":["art:2"],"body":"Approve?","gate":true}}'
  } | "$BBP_BIN" mcp
fi
"##;

/// A confined session keeps its home across turns, so its conversation
/// survives, while every turn starts in an empty workspace at one path.
#[test]
#[ignore = "requires enforced Linux sandbox; required acceptance job"]
fn a_confined_session_keeps_its_home_and_one_fresh_workspace_across_turns() {
    let root = root("session");
    let mut cfg = setup(&root, "T1", SESSION, vec![], Some(scripts(&root)));
    let mut l = cfg.launchers.iter().next().unwrap().1.clone();
    l.session = true;
    cfg.launchers = Launchers::default().with("planner", l);
    let (dir, task) = (cfg.dir.clone(), cfg.task.clone());
    plan(cfg);
    let id = moderator::session_id(
        &rusty_bbp_host::master::load(&dir).unwrap(),
        &task,
        &Who::Planner.id(),
    );
    let workspace = root
        .join("work/turns")
        .join(Sha256::of(task.0.as_bytes()).hex())
        .join(format!("session-{id}"));
    for turn in 1..=4u64 {
        let said = std::fs::read_to_string(moderator::log_path(&dir, &task, TurnId(turn))).unwrap();
        println!("{said}");
        let resume = if turn == 1 { "no" } else { "yes" };
        let line = format!(
            "turn {turn} resume {resume} cwd {} left 0",
            workspace.display()
        );
        assert!(said.contains(&line), "{line}:\n{said}");
        assert_eq!(
            moderator::resumed_path(&dir, &task, TurnId(turn)).exists(),
            turn > 1,
            "turn {turn}"
        );
    }
    let _ = std::fs::remove_dir_all(&root);
    println!("BBP_ACCEPTANCE PASS session-confined");
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
/// then waits for the host to release both probe participants before planning.
const NEIGHBOUR: &str = r##"#!/bin/sh
barrier="$1"; repo="$2"; attempts="$3"; shift 3
wait_for() {
  count=0
  until test "$1" "$2"; do
    if [ "$count" -ge "$attempts" ]; then
      echo "barrier timeout: task=$BBP_TASK test=$1 path=$2 attempts=$attempts" >&2
      exit 1
    fi
    count=$((count + 1))
    sleep 0.05
  done
}
echo "barrier: ready"
wait_for -f "$barrier/$BBP_TASK.start"
for w in "$@"; do wait_for -d "$w"; done
echo "common: $(git rev-parse --path-format=absolute --git-common-dir)"
echo "flag: $(cat flag.txt)"
for w in "$@"; do
  if echo x > "$w/from-$BBP_TASK" 2>/dev/null; then echo "wrote: $w"; else echo "denied: $w"; fi
done
if echo x > "$repo/.git/objects/forged" 2>/dev/null; then echo "objects: wrote"; else echo "objects: denied"; fi
echo "barrier: probes complete"
# Do not exit: the moderator reaps exited harnesses even before cancellation.
wait_for -f "$barrier/release"
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

fn neighbour_args(barrier: &Path, repo: &Path, one: &Path, two: &Path) -> Vec<String> {
    vec![
        barrier.to_string_lossy().into_owned(),
        repo.to_string_lossy().into_owned(),
        "400".into(), // 20 seconds per wait, bounded below the moderator's wall limit.
        one.to_string_lossy().into_owned(),
        two.to_string_lossy().into_owned(),
    ]
}

fn wait_for_probe_logs(logs: &[PathBuf; 2], marker: &str, participants: &[usize]) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let said = logs
            .each_ref()
            .map(|p| std::fs::read_to_string(p).unwrap_or_default());
        if participants
            .iter()
            .all(|i| said[*i].lines().any(|line| line == marker))
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "probe barrier timed out waiting for {marker:?} from {participants:?}; logs {logs:?}:\nT1:\n{}\nT2:\n{}",
            said[0], said[1]
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Force the old race's ordering without a timing assumption: T2 cannot probe
/// until T1 has finished probing. Both must stay alive until the host releases them.
fn release_delayed_probes(barrier: &Path, worktrees: [&Path; 2], logs: &[PathBuf; 2]) {
    wait_for_probe_logs(logs, "barrier: ready", &[0, 1]);
    std::fs::write(barrier.join("T1.start"), b"").expect("start T1");
    wait_for_probe_logs(logs, "barrier: probes complete", &[0]);
    assert!(
        worktrees.iter().all(|w| w.is_dir()),
        "worktrees disappeared before T2 probed: {worktrees:?}"
    );
    assert!(!std::fs::read_to_string(&logs[1])
        .expect("T2 log")
        .contains("barrier: probes complete"));
    std::fs::write(barrier.join("T2.start"), b"").expect("start T2");
    wait_for_probe_logs(logs, "barrier: probes complete", &[0, 1]);
    assert!(
        worktrees.iter().all(|w| w.is_dir()),
        "worktrees disappeared before barrier release: {worktrees:?}"
    );
    std::fs::write(barrier.join("release"), b"").expect("release probes");
}

#[test]
fn neighbour_barrier_keeps_a_fast_fixture_alive_until_the_delayed_peer_probes() {
    // Exercise the actual harness script without requiring Landlock. Reap each
    // fixture's workspace immediately on exit, just as Turn::finish does.
    let root = root("delayed-fixture");
    let barrier = root.join("barrier");
    let repo = root.join("repo");
    std::fs::create_dir_all(&barrier).expect("barrier");
    std::fs::create_dir_all(repo.join(".git/objects")).expect("objects");
    let worktrees = [root.join("one"), root.join("two")];
    let logs = [root.join("T1.log"), root.join("T2.log")];
    let runs: Vec<_> = ["T1", "T2"]
        .into_iter()
        .enumerate()
        .map(|(i, name)| {
            std::fs::create_dir_all(&worktrees[i]).expect("worktree");
            std::fs::write(worktrees[i].join("flag.txt"), "no\n").expect("flag");
            let log = std::fs::File::create(&logs[i]).expect("log");
            let mut child = std::process::Command::new("/bin/sh")
                .args(["-c", NEIGHBOUR, "neighbour"])
                .args(neighbour_args(
                    &barrier,
                    &repo,
                    &worktrees[0],
                    &worktrees[1],
                ))
                .env("BBP_TASK", name)
                .env("BBP_BIN", "/bin/true")
                .current_dir(&worktrees[i])
                .stdout(log.try_clone().expect("clone log"))
                .stderr(log)
                .spawn()
                .expect("fixture");
            let own = worktrees[i].clone();
            std::thread::spawn(move || {
                let status = child.wait().expect("wait");
                std::fs::remove_dir_all(own).expect("reap worktree");
                assert!(status.success(), "fixture {name}: {status}");
            })
        })
        .collect();
    release_delayed_probes(&barrier, [&worktrees[0], &worktrees[1]], &logs);
    for run in runs {
        run.join().expect("fixture join");
    }
    assert!(worktrees.iter().all(|w| !w.exists()));
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn neighbour_barrier_reports_missing_start_worktree_and_release() {
    let root = root("barrier-timeout");
    let missing_worktree = root.join("missing-worktree");
    for phase in ["start", "worktree", "release"] {
        let mut command = std::process::Command::new("/bin/sh");
        command
            .args(["-c", NEIGHBOUR, "neighbour"])
            .arg(&root)
            .arg(&root)
            .arg("1")
            .env("BBP_TASK", "missing");
        if phase != "start" {
            std::fs::write(root.join("missing.start"), b"").expect("start");
        }
        if phase == "worktree" {
            command.arg(&missing_worktree);
        }
        let output = command.output().expect("fixture");
        assert!(!output.status.success(), "{phase} wait must fail closed");
        let error = String::from_utf8(output.stderr).expect("stderr");
        let (test, path) = match phase {
            "start" => ("-f", root.join("missing.start")),
            "worktree" => ("-d", missing_worktree.clone()),
            _ => ("-f", root.join("release")),
        };
        assert!(
            error.contains(&format!(
                "barrier timeout: task=missing test={test} path={} attempts=1",
                path.display()
            )),
            "{phase}: {error}"
        );
    }
    std::fs::remove_dir_all(root).expect("cleanup");
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
    // Control files live beside the scripts: read-only to both harnesses.
    let barrier = root.join("bin/barrier");
    std::fs::create_dir_all(&barrier).expect("barrier");
    let args = neighbour_args(&barrier, &root.join("repo"), &one, &two);
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
    let logs = ["T1", "T2"]
        .map(|name| moderator::log_path(&root.join(name), &TaskId(name.into()), TurnId(1)));
    release_delayed_probes(&barrier, [&one, &two], &logs);
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

/// A one-commit repository at `repo` with `files`.
fn commit_repo(repo: &Path, files: &[(&str, &str)]) {
    for (path, text) in files {
        let path = repo.join(path);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, text).expect("write");
    }
    git(repo, &["init", "-q", "-b", "main"]);
    git(repo, &["add", "."]);
    let who = ["-c", "user.name=t", "-c", "user.email=t@example.invalid"];
    git(repo, &[&who[..], &["commit", "-qm", "base"]].concat());
}

/// Task `T1` on `repo` with the example roster, its maker the built-in
/// Codex launcher running `script` (with `TASK_DIR` and `REPO` set) as the
/// provider, confined by `sandbox`, whose read roots gain the script's.
fn codex_task(root: &Path, repo: &Path, script: &str, mut sandbox: HarnessSandbox) -> Config {
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
        &ProfileSet::shell("test", "grep -q '^ok$' flag.txt"),
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
    let script = script.replacen(
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
    sandbox.read.push(bin.to_string_lossy().into_owned());
    let launcher = Launcher {
        program: env!("CARGO_BIN_EXE_codex-bbp-harness").into(),
        args: vec![
            "--provider-bin".into(),
            stub.to_string_lossy().into_owned(),
            "--mcp-config".into(),
            "{mcp_config}".into(),
        ],
        builtin_tools: true,
        session: false,
        sandbox: Some(sandbox),
    };
    Config {
        dir,
        task,
        bbp: PathBuf::from(env!("CARGO_BIN_EXE_bbp")),
        repo: repo.to_path_buf(),
        work: root.join("work"),
        launchers: Launchers::default().with("maker", launcher),
        confinement: Confinement::Unconfined,
        poll: Duration::from_millis(50),
        max_wall: Duration::from_secs(60),
        end_grace: Duration::from_secs(5),
        deadline: None,
        cost_cap: None,
        admission: None,
    }
}

#[test]
#[ignore = "requires enforced Linux sandbox; required acceptance job"]
fn a_confined_codex_launcher_checks_its_turn_and_plans_from_a_shared_clone() {
    let root = root("launcher").canonicalize().expect("canonical");
    let repo = root.join("repo");
    commit_repo(&repo, &[("flag.txt", "no\n")]);
    let sandbox = HarnessSandbox {
        read: vec![repo.join(".git").to_string_lossy().into_owned()],
        ..HarnessSandbox::default()
    };
    let cfg = codex_task(&root, &repo, AUTHOR, sandbox);
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

/// Reports what it sees of the filtered view, then posts a diff that adds a
/// file under the hidden directory and prints the answer.
const HIDDEN: &str = r##"#!/bin/sh
secret="$1"
if [ -e secrets ]; then echo "secrets: seen"; else echo "secrets: absent"; fi
echo "flag: $(cat flag.txt)"
if git cat-file -e "$secret" 2>/dev/null; then echo "blob: leaked"; else echo "blob: absent"; fi
if git diff --quiet; then echo "view: clean"; else echo "view: dirty"; fi
"$BBP_BIN" mcp <<'RPC'
{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"sh","version":"0"}}}
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"put_artifact","arguments":{"op":"d1","kind":"diff","body":"diff --git a/secrets/new.txt b/secrets/new.txt\nnew file mode 100644\n--- /dev/null\n+++ b/secrets/new.txt\n@@ -0,0 +1 @@\n+x\n"}}}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"put_artifact","arguments":{"op":"p1","kind":"spec","brief":1,"body":"# Spec\nAC1 flag says ok."}}}
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"post","arguments":{"op":"p2","kind":"request_decision","to":["human"],"refs":["art:2"],"body":"Approve the plan?","gate":true}}}
RPC
"##;

#[test]
#[ignore = "requires enforced Linux sandbox; required acceptance job"]
fn a_hiding_harness_sees_a_filtered_copy_and_cannot_post_into_what_it_hides() {
    let root = root("hidden");
    let repo = root.join("repo");
    std::fs::create_dir_all(repo.join("secrets")).expect("mkdir");
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("flag.txt"), "no\n").expect("write");
    std::fs::write(repo.join("secrets/token.txt"), "hunter2\n").expect("write");
    git(&repo, &["add", "."]);
    let who = ["-c", "user.name=t", "-c", "user.email=t@example.invalid"];
    git(&repo, &[&who[..], &["commit", "-qm", "base"]].concat());
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(&repo)
        .args(["rev-parse", "HEAD:secrets/token.txt"])
        .output()
        .expect("git");
    let secret = String::from_utf8(out.stdout)
        .expect("utf8")
        .trim()
        .to_owned();
    let root = root.canonicalize().expect("canonical");
    let sandbox = HarnessSandbox {
        checkout: true,
        hide: vec!["secrets".into()],
        ..scripts(&root)
    };
    let said = plan(setup(&root, "T1", HIDDEN, vec![secret], Some(sandbox)));
    println!("{said}");
    for line in [
        "secrets: absent",
        "flag: no",
        "blob: absent",
        "view: clean",
        "ProtectedPath",
        "paths hidden from this agent: secrets/new.txt",
    ] {
        assert!(said.contains(line), "{line}:\n{said}");
    }
    let workspace = root
        .join("work/turns")
        .join(Sha256::of(b"T1").hex())
        .join("turn-1");
    assert!(
        !workspace.exists(),
        "the filtered copy is removed after the turn"
    );
    let _ = std::fs::remove_dir_all(&root);
    println!("BBP_ACCEPTANCE PASS filtered-view");
}

#[test]
#[ignore = "requires enforced Linux sandbox; required acceptance job"]
fn a_dry_run_reads_its_real_data_and_writes_only_its_plan() {
    let root = root("dry_run").canonicalize().expect("canonical");
    let data = root.join("data");
    std::fs::create_dir_all(&data).expect("mkdir");
    std::fs::write(data.join("a.csv"), "row\n").expect("write");
    let script = r#"cat "$BBP_DATA_CRM/a.csv" > /dev/null || exit 7
if echo x > "$BBP_DATA_CRM/forged" 2>/dev/null; then exit 8; fi
if cat /etc/hostname > /dev/null 2>&1; then exit 9; fi
printf '{"set":1}\n{"set":2}\n' > "$BBP_PLAN""#;
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_bbp"))
        .args(["dry-run", "--name", "crm-fix", "--data"])
        .arg(format!("crm={}", data.display()))
        .arg("--work")
        .arg(root.join("work"))
        .args(["--", "/bin/sh", "-c", script])
        .output()
        .expect("bbp");
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{said}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(said.contains("\"entries\":2"), "{said}");
    assert!(
        !said.contains("\"set\""),
        "the plan itself is never printed: {said}"
    );
    assert!(!data.join("forged").exists());
    // What a failed or timed-out run printed may be real data: never shown.
    std::fs::write(data.join("secret.csv"), "ssn=078-05-1120\n").expect("write");
    let leak = r#"cat "$BBP_DATA_CRM/secret.csv"; cat "$BBP_DATA_CRM/secret.csv" >&2"#;
    for (case, wall, tail) in [("failed", "30", "exit 5"), ("timed out", "1", "sleep 30")] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_bbp"))
            .args(["dry-run", "--name", "leak", "--wall-secs", wall, "--data"])
            .arg(format!("crm={}", data.display()))
            .arg("--work")
            .arg(root.join("work"))
            .args(["--", "/bin/sh", "-c", &format!("{leak}; {tail}")])
            .output()
            .expect("bbp");
        let shown = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(!out.status.success(), "{case}: {shown}");
        assert!(!shown.contains("078-05-1120"), "{case} leaked: {shown}");
    }
    // Links planted where the host once wrote the output, and into the
    // host-side output directory, redirect none of the host's writes.
    std::fs::write(data.join("keep"), "real").expect("write");
    let plant = r#"for f in stdout.log stderr.log; do ln -s "$BBP_DATA_CRM/keep" "$HOME/$f"; done
if ln -s "$BBP_DATA_CRM/keep" "$HOME/../plant.out/stdout.log" 2>/dev/null; then echo "out: planted" >&2; fi
echo overwritten; echo overwritten >&2; exit 3"#;
    for _ in 0..2 {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_bbp"))
            .args(["dry-run", "--name", "plant", "--data"])
            .arg(format!("crm={}", data.display()))
            .arg("--work")
            .arg(root.join("work"))
            .args(["--", "/bin/sh", "-c", plant])
            .output()
            .expect("bbp");
        assert!(!out.status.success());
        assert_eq!(
            std::fs::read_to_string(data.join("keep")).expect("read"),
            "real",
            "the data is untouched"
        );
        let err = std::fs::read_to_string(root.join("work/dry-run/plant.out/stderr.log"))
            .expect("kept host-side");
        assert!(!err.contains("out: planted"), "{err}");
    }
    let _ = std::fs::remove_dir_all(&root);
    println!("BBP_ACCEPTANCE PASS dry-run");
}

/// A Codex provider stand-in for a filtered author: plans on turn 1; on
/// turn 2 reports what it sees, makes the flag say ok, and submits it, first
/// on a base the copy never had (refused), then on its clone's `HEAD`.
const FILTERED_AUTHOR: &str = r##"#!/bin/sh
cfg=$(for a in "$@"; do case "$a" in mcp_servers=*) printf '%s' "$a";; esac; done)
relay=$(printf '%s' "$cfg" | sed -n 's/.*"BBP_MCP_RELAY"="\([^"]*\)".*/\1/p')
bbp=$(printf '%s' "$cfg" | sed -n 's/^mcp_servers={bbp={command="\([^"]*\)".*/\1/p')
rpc() {
  { printf '%s\n' '{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"sh","version":"0"}}}'; cat; } |
    BBP_MCP_RELAY="$relay" "$bbp" mcp
}
case "$*" in
*"turn 1."*)
  rpc <<'RPC'
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"put_artifact","arguments":{"op":"t1-p1","kind":"spec","brief":1,"body":"# Spec\nAC1 flag says ok."}}}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"post","arguments":{"op":"t1-p2","kind":"request_decision","to":["human"],"refs":["art:2"],"body":"Approve the plan?","gate":true}}}
RPC
  ;;
*"turn 2."*)
  if [ -e secrets ]; then echo "secrets: seen"; else echo "secrets: absent"; fi
  if cat "$REPO/secrets/token.txt" >/dev/null 2>&1; then echo "source: read"; else echo "source: denied"; fi
  echo ok > flag.txt
  base=$(git rev-parse HEAD)
  diff=$(git diff | sed 's/\\/\\\\/g; s/"/\\"/g' | awk '{printf "%s\\n", $0}')
  rpc <<RPC
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"put_artifact","arguments":{"op":"t2-d1","kind":"diff","body":"$diff"}}}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"put_artifact","arguments":{"op":"t2-c0","kind":"candidate","base":"0123456789012345678901234567890123456789","diffs":[3]}}}
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"put_artifact","arguments":{"op":"t2-c1","kind":"candidate","base":"$base","diffs":[3]}}}
RPC
  ;;
esac
"##;

/// The source repository for the filtered tests: a flag, the test of it,
/// and a secret under `secrets/`.
fn secret_repo(repo: &Path) {
    commit_repo(
        repo,
        &[("flag.txt", "no\n"), ("secrets/token.txt", "hunter2\n")],
    );
}

#[test]
#[ignore = "requires enforced Linux sandbox; required acceptance job"]
fn a_filtered_codex_author_builds_a_candidate_the_runner_runs_on_the_source_commit() {
    let root = root("filtered_build").canonicalize().expect("canonical");
    let repo = root.join("repo");
    secret_repo(&repo);
    let sandbox = HarnessSandbox {
        checkout: true,
        hide: vec!["secrets".into()],
        ..HarnessSandbox::default()
    };
    let cfg = codex_task(&root, &repo, FILTERED_AUTHOR, sandbox);
    let (dir, task) = (cfg.dir.clone(), cfg.task.clone());
    let moderator = std::thread::spawn(move || moderator::run(&cfg));
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let card = admin::card(&dir, &task).expect("card");
        if card.state == State::PlanGate {
            human::perform(&dir, &task, "approve-plan", &["2"], None).expect("approve plan");
        }
        let d = rusty_bbp_host::open_driver(&dir, &task).expect("driver");
        if d.state.selected_run().is_some_and(|r| r.status.is_some()) {
            let card = admin::card(&dir, &task).expect("card");
            human::perform(&dir, &task, "cancel", &["done"], Some(card.rev)).expect("cancel");
            break;
        }
        assert!(
            !moderator.is_finished() && Instant::now() < deadline,
            "no finished run; turn 2 said:\n{}",
            std::fs::read_to_string(moderator::log_path(&dir, &task, TurnId(2)))
                .unwrap_or_default()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    moderator.join().expect("join").expect("moderator");
    let said = std::fs::read_to_string(moderator::log_path(&dir, &task, TurnId(2))).expect("log");
    println!("{said}");
    for line in ["secrets: absent", "source: denied", "RefUnresolved"] {
        assert!(said.contains(line), "{line}:\n{said}");
    }
    let source = String::from_utf8(
        std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["rev-parse", "HEAD"])
            .output()
            .expect("git")
            .stdout,
    )
    .expect("utf8");
    let d = rusty_bbp_host::open_driver(&dir, &task).expect("driver");
    let run = d.state.selected_run().expect("run");
    assert_eq!(run.status, Some(RunStatus::Passed), "the flag test passed");
    let cand = d
        .state
        .artifacts
        .values()
        .find_map(|a| match &a.payload {
            ArtifactPayload::Candidate(m) => Some(m.base.clone()),
            _ => None,
        })
        .expect("one candidate");
    assert_eq!(cand, source.trim(), "pinned to the source commit");
    let _ = std::fs::remove_dir_all(&root);
    println!("BBP_ACCEPTANCE PASS filtered-build");
}

#[test]
fn a_hiding_sandbox_is_refused_when_an_implicit_root_reaches_the_repository() {
    let root = root("hidden_implicit").canonicalize().expect("canonical");
    let repo = root.join("repo");
    secret_repo(&repo);
    // `bbp` installed inside the repository: every harness reads its directory.
    let tools = repo.join("tools");
    std::fs::create_dir_all(&tools).expect("mkdir");
    let bbp = tools.join("bbp");
    std::fs::copy(env!("CARGO_BIN_EXE_bbp"), &bbp).expect("copy bbp");
    let sandbox = HarnessSandbox {
        checkout: true,
        hide: vec!["secrets".into()],
        ..scripts(&root)
    };
    let mut cfg = setup(&root, "T1", HIDDEN, vec!["x".into()], Some(sandbox));
    cfg.bbp = bbp;
    let error = moderator::run(&cfg).expect_err("the moderator refuses to launch");
    assert!(error.contains("reaches"), "{error}");
    assert!(planner_log(&cfg.dir, &cfg.task).is_empty(), "nothing ran");
    assert!(
        !root.join("work/turns").exists(),
        "no filtered copy was made"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_hiding_sandbox_is_refused_when_it_reaches_a_linked_git_directory() {
    let root = root("hidden_linked_git").canonicalize().expect("canonical");
    let repo = root.join("repo");
    secret_repo(&repo);
    // A supported layout: `.git` a link to a backing directory elsewhere.
    let backing = root.join("backing-git");
    std::fs::rename(repo.join(".git"), &backing).expect("move .git");
    std::os::unix::fs::symlink(&backing, repo.join(".git")).expect("link .git");
    let mut sandbox = HarnessSandbox {
        checkout: true,
        hide: vec!["secrets".into()],
        ..scripts(&root)
    };
    sandbox.read.push(backing.to_string_lossy().into_owned());
    let cfg = setup(&root, "T1", HIDDEN, vec!["x".into()], Some(sandbox));
    let error = moderator::run(&cfg).expect_err("the moderator refuses to launch");
    assert!(error.contains("backing-git"), "{error}");
    assert!(planner_log(&cfg.dir, &cfg.task).is_empty(), "nothing ran");
    assert!(
        !root.join("work/turns").exists(),
        "no filtered copy was made"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// A supported, composed layout: `.git/objects` linked to an object store
/// whose `pack` and one loose-object directory are linked again elsewhere.
/// A harness granted only one of those final places is refused before
/// anything runs or is copied.
#[test]
fn a_hiding_sandbox_is_refused_when_it_reaches_storage_linked_twice() {
    let base = root("hidden_linked_twice")
        .canonicalize()
        .expect("canonical");
    let layout = |root: &Path| -> (PathBuf, PathBuf) {
        let repo = root.join("repo");
        secret_repo(&repo);
        let store = root.join("object-store");
        std::fs::rename(repo.join(".git/objects"), &store).expect("move objects");
        std::os::unix::fs::symlink(&store, repo.join(".git/objects")).expect("link objects");
        let packs = root.join("separate-packs");
        std::fs::rename(store.join("pack"), &packs).expect("move pack");
        std::os::unix::fs::symlink(&packs, store.join("pack")).expect("link pack");
        let fan = std::fs::read_dir(&store)
            .expect("store")
            .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
            .find(|n| n.len() == 2 && n.chars().all(|c| c.is_ascii_hexdigit()))
            .expect("a loose-object directory");
        let loose = root.join("separate-loose");
        std::fs::rename(store.join(&fan), &loose).expect("move loose");
        std::os::unix::fs::symlink(&loose, store.join(&fan)).expect("link loose");
        (packs, loose)
    };
    for (case, pick) in [("packs", 0), ("loose", 1)] {
        let root = base.join(case);
        std::fs::create_dir_all(&root).expect("mkdir");
        let (packs, loose) = layout(&root);
        let granted = [&packs, &loose][pick];
        let mut sandbox = HarnessSandbox {
            checkout: true,
            hide: vec!["secrets".into()],
            ..scripts(&root)
        };
        sandbox.read.push(granted.to_string_lossy().into_owned());
        let cfg = setup(&root, "T1", HIDDEN, vec!["x".into()], Some(sandbox));
        let error = moderator::run(&cfg).expect_err("the moderator refuses to launch");
        let name = granted.file_name().expect("name").to_string_lossy();
        assert!(error.contains(name.as_ref()), "{case}: {error}");
        assert!(
            planner_log(&cfg.dir, &cfg.task).is_empty(),
            "{case}: nothing ran"
        );
        assert!(
            !root.join("work/turns").exists(),
            "{case}: no filtered copy was made"
        );
    }
    let _ = std::fs::remove_dir_all(&base);
}

/// An alternate whose path is not UTF-8, beneath a UTF-8 parent the harness
/// is granted: git reads `alternates` as bytes, so the borrowed store is
/// real, and the check cannot read it as text. It refuses before anything
/// runs or is copied rather than dropping the store.
#[test]
fn a_hiding_sandbox_is_refused_when_an_alternate_is_not_utf8() {
    use std::os::unix::ffi::OsStrExt;
    let root = root("hidden_byte_alternate")
        .canonicalize()
        .expect("canonical");
    let repo = root.join("repo");
    secret_repo(&repo);
    let parent = root.join("external-stores");
    let store = parent.join(std::ffi::OsStr::from_bytes(b"store-\xff"));
    std::fs::create_dir_all(&store).expect("mkdir store");
    let mut line = store.as_os_str().as_bytes().to_vec();
    line.push(b'\n');
    std::fs::write(repo.join(".git/objects/info/alternates"), line).expect("alternates");
    let mut sandbox = HarnessSandbox {
        checkout: true,
        hide: vec!["secrets".into()],
        ..scripts(&root)
    };
    sandbox.read.push(parent.to_string_lossy().into_owned());
    let cfg = setup(&root, "T1", HIDDEN, vec!["x".into()], Some(sandbox));
    let error = moderator::run(&cfg).expect_err("the moderator refuses to launch");
    assert!(error.contains("alternates"), "{error}");
    assert!(planner_log(&cfg.dir, &cfg.task).is_empty(), "nothing ran");
    assert!(
        !root.join("work/turns").exists(),
        "no filtered copy was made"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Git splits `alternates` at LF only, so a CRLF entry borrows the store
/// whose name ends in CR, not its CR-less twin. A harness granted that
/// store is refused before anything runs or is copied.
#[test]
fn a_hiding_sandbox_is_refused_when_a_crlf_alternate_names_its_grant() {
    let root = root("hidden_crlf_alternate")
        .canonicalize()
        .expect("canonical");
    let repo = root.join("repo");
    secret_repo(&repo);
    let plain = root.join("external").join("store");
    let cr = root.join("external").join("store\r");
    std::fs::create_dir_all(&plain).expect("mkdir store");
    std::fs::create_dir_all(&cr).expect("mkdir store CR");
    std::fs::write(
        repo.join(".git/objects/info/alternates"),
        format!("{}\r\n", plain.display()),
    )
    .expect("alternates");
    let mut sandbox = HarnessSandbox {
        checkout: true,
        hide: vec!["secrets".into()],
        ..scripts(&root)
    };
    sandbox.read.push(cr.to_string_lossy().into_owned());
    let cfg = setup(&root, "T1", HIDDEN, vec!["x".into()], Some(sandbox));
    let error = moderator::run(&cfg).expect_err("the moderator refuses to launch");
    assert!(error.contains("reaches"), "{error}");
    assert!(planner_log(&cfg.dir, &cfg.task).is_empty(), "nothing ran");
    assert!(
        !root.join("work/turns").exists(),
        "no filtered copy was made"
    );
    let _ = std::fs::remove_dir_all(&root);
}
