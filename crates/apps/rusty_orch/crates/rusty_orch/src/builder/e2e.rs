//! #1318 S6: `rusty_orch builder run` against a real mux server, real
//! `lel-jail` and `bbp`, a delegated cgroup, and a scripted fake ACP agent
//! in place of `claude-code-acp` (no model call). Run by the Orch BBP
//! confined acceptance job, which builds the binaries and delegates the
//! cgroup (design §11, slice design §S6).
//!
//! Each test needs `BBP_TEST_BIN`, `ORCH_TEST_BIN`, `MUX_SERVER_TEST_BIN`,
//! `MUX_CTL_TEST_BIN`, `JAIL_TEST_BIN` and `LEL_TEST_CGROUP`, a cgroup2
//! directory this user may create cgroups in and move its own processes
//! between.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child as Process, Command, Output, Stdio};
use std::time::{Duration, Instant};

use orch_lens::{LensConfig, Post};
use rusty_bbp::{ClaimBody, MsgId, Stakes};
use rusty_json::Value;

use crate::lens::bbp::{run_id, Child, Session};
use orch_lens::round::Board;

const CLAIM_TEXT: &str = "Add a greeting file";

/// The fake `claude-code-acp`: answers `initialize` and `session/new`;
/// on the prompt, probes the jail into `probe.json`, edits the worktree,
/// writes `.lel/submit` and ends the turn, or, when `$FAKE/mode` says
/// `hang`, writes `.prompted` in the worktree and hangs.
const AGENT: &str = r#"#!/usr/bin/python3
import json, os, socket, sys, time
FAKE = os.path.dirname(os.path.realpath(__file__))
def send(m):
    sys.stdout.write(json.dumps(m) + "\n"); sys.stdout.flush()
def mode():
    try: return open(os.path.join(FAKE, "mode")).read().strip()
    except OSError: return ""
def probe():
    out = {"mux_env": "RUSTY_MUX_SOCKET" in os.environ}
    try:
        socket.socket(socket.AF_UNIX, socket.SOCK_STREAM).close(); out["unix_socket"] = "allowed"
    except OSError as e:
        out["unix_socket"] = "refused: %s" % e
    try:
        open(os.path.join(FAKE, "escape"), "w").close(); out["write_outside"] = "allowed"
    except OSError as e:
        out["write_outside"] = "refused: %s" % e
    return out
for line in sys.stdin:
    msg = json.loads(line)
    method, mid = msg.get("method"), msg.get("id")
    if method == "initialize":
        send({"jsonrpc": "2.0", "id": mid, "result": {"protocolVersion": 1}})
    elif method == "session/new":
        send({"jsonrpc": "2.0", "id": mid, "result": {"sessionId": "s1"}})
    elif method == "session/prompt":
        if mode() == "hang":
            open(os.path.join(os.getcwd(), ".prompted"), "w").close()
            while True: time.sleep(1)
        cwd = os.getcwd()
        json.dump(probe(), open(os.path.join(cwd, "probe.json"), "w"))
        open(os.path.join(cwd, "greeting.txt"), "w").write("hello\n")
        os.makedirs(os.path.join(cwd, ".lel"), exist_ok=True)
        open(os.path.join(cwd, ".lel", "submit"), "w").write("done\n")
        send({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "s1",
              "update": {"sessionUpdate": "agent_message_chunk",
                         "content": {"type": "text", "text": "fake builder wrote greeting.txt\n"}}}})
        send({"jsonrpc": "2.0", "id": mid, "result": {"stopReason": "end_turn",
              "usage": {"inputTokens": 11, "outputTokens": 7}}})
"#;

fn bin(var: &str) -> PathBuf {
    let path = std::env::var_os(var).unwrap_or_else(|| panic!("{var} is required"));
    PathBuf::from(path).canonicalize().expect(var)
}

fn run(cmd: &mut Command) -> Output {
    let out = cmd.output().expect("run");
    assert!(
        out.status.success() && !String::from_utf8_lossy(&out.stdout).contains("Rejected"),
        "{cmd:?}: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

fn git(repo: &Path, args: &[&str]) -> String {
    let out = run(Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@example.invalid"])
        .args(args)
        .current_dir(repo));
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// Everything one test sets up, removed on drop.
struct World {
    root: PathBuf,
    bbp: PathBuf,
    dir: PathBuf,
    repo: PathBuf,
    base: String,
    config: PathBuf,
    fake: PathBuf,
    home: PathBuf,
    socket: PathBuf,
    server: Option<Process>,
    cgroup_parent: PathBuf,
    claim: MsgId,
}

const TASK: &str = "toy";
const LENS_CONFIG: &str = r#"{
  "providers": {
    "codex": {"kind": "codex-cli", "context_tokens": 32768,
              "price": {"input": 0, "output": 0, "cached_input": 0}}
  },
  "ceilings": {"experiment_usd": 0, "task_usd": 0, "task_calls": 10},
  "builders": {"builders_per_claim": 1, "max_live_builders": 3, "builder_wall_secs": 600},
  "lenses": [
    {"id": "architect", "prompt": "p", "domains": ["*"],
     "may_post": {"claim": true},
     "models": {"routine": ["codex"], "contested": ["codex"]}}
  ]
}"#;

impl World {
    fn new(tag: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("orch-builder-e2e-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("root");
        let bbp = bin("BBP_TEST_BIN");
        let repo = root.join("repo");
        std::fs::create_dir_all(&repo).expect("repo");
        std::fs::write(repo.join("README"), "toy\n").expect("readme");
        git(&repo, &["init", "-q"]);
        git(&repo, &["add", "README"]);
        git(&repo, &["commit", "-qm", "base"]);
        let base = git(&repo, &["rev-parse", "HEAD"]);
        let dir = open_task(&root, &bbp, &repo);
        let config = root.join("lens.json");
        std::fs::write(&config, LENS_CONFIG).expect("config");
        std::fs::write(root.join("p"), "prompt").expect("prompt");
        let fake = root.join("fake");
        std::fs::create_dir_all(&fake).expect("fake");
        let agent = fake.join("claude-code-acp");
        std::fs::write(&agent, AGENT).expect("agent");
        std::fs::set_permissions(&agent, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        let home = root.join("home");
        std::fs::create_dir_all(home.join(".claude")).expect("home");
        let socket = root.join("mux.sock");
        let scope = run(Command::new(&bbp).args(["cgroup-scope", "--dir"]).arg(&dir));
        let scope = String::from_utf8_lossy(&scope.stdout).trim().to_owned();
        let cgroup_parent = bin("LEL_TEST_CGROUP").join(scope);
        std::fs::create_dir_all(&cgroup_parent).expect("scope cgroup");
        let claim = post_claim(&bbp, &dir, &config);
        let mut w = Self {
            root,
            bbp,
            dir,
            repo,
            base,
            config,
            fake,
            home,
            socket,
            server: None,
            cgroup_parent,
            claim,
        };
        w.start_mux();
        w
    }

    fn start_mux(&mut self) {
        let server = Command::new(bin("MUX_SERVER_TEST_BIN"))
            .arg("--no-state")
            .arg(&self.socket)
            .env("HOME", &self.home)
            .stdin(Stdio::null())
            .spawn()
            .expect("mux server");
        self.server = Some(server);
        let until = Instant::now() + Duration::from_secs(20);
        while !self.socket.exists() {
            assert!(Instant::now() < until, "mux server never bound");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn kill_mux(&mut self) {
        if let Some(mut s) = self.server.take() {
            let _ = s.kill();
            let _ = s.wait();
        }
    }

    fn state(&self) -> PathBuf {
        self.root.join("state")
    }

    fn slots(&self) -> PathBuf {
        let slots = self.root.join("slots");
        std::fs::create_dir_all(&slots).expect("slots");
        std::fs::write(slots.join("capacity"), "agents=1 validators=1\n").expect("capacity");
        slots
    }

    fn builder(&self) -> Command {
        let path = format!(
            "{}:{}",
            self.fake.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let mut cmd = Command::new(bin("ORCH_TEST_BIN"));
        cmd.arg("builder")
            .arg("run")
            .arg(&self.config)
            .arg("--bbp")
            .arg(&self.bbp)
            .arg("--dir")
            .arg(&self.dir)
            .args(["--task", TASK, "--claim", &self.claim.0.to_string()])
            .arg("--state")
            .arg(self.state())
            .arg("--repo")
            .arg(&self.repo)
            .args(["--base", &self.base])
            .arg("--cgroup-parent")
            .arg(&self.cgroup_parent)
            .arg("--slots")
            .arg(self.slots())
            .arg("--mux-ctl")
            .arg(bin("MUX_CTL_TEST_BIN"))
            .arg("--mux-socket")
            .arg(&self.socket)
            .arg("--jail")
            .arg(bin("JAIL_TEST_BIN"))
            .arg("--read")
            .arg(&self.fake)
            .env("PATH", path)
            .env("HOME", &self.home)
            .env_remove("RUSTY_MUX_SOCKET")
            .stdin(Stdio::null());
        cmd
    }

    fn worktree(&self) -> PathBuf {
        self.state()
            .join("wt")
            .join(self.claim.0.to_string())
            .join("1")
    }

    fn run_dir(&self) -> PathBuf {
        self.state()
            .join("builders")
            .join(self.claim.0.to_string())
            .join("1")
    }

    /// The claim's card as the builder's seat sees it.
    fn card(&self) -> orch_lens::card::Card {
        let config = LensConfig::parse(LENS_CONFIG).expect("config");
        let child = Child::spawn(&self.bbp, &self.dir, TASK, "claude-builder").expect("seat");
        let mut seat =
            Session::new("claude-builder", child, &run_id().expect("run id")).follow_every_domain();
        seat.refresh(&config).expect("refresh");
        seat.cards(&config)
            .into_iter()
            .find(|c| c.id == self.claim)
            .expect("the claim's card")
    }

    /// Whether the only agent slot can be taken now.
    fn slot_free(&self) -> bool {
        Command::new("flock")
            .args(["-n", "-x"])
            .arg(self.slots().join("agent-0.lock"))
            .arg("true")
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

impl Drop for World {
    fn drop(&mut self) {
        self.kill_mux();
        // Kill whatever a failed case left running below the scope, so the
        // rmdirs below can succeed.
        let _ = std::fs::write(self.cgroup_parent.join("cgroup.kill"), "1");
        for _ in 0..50 {
            if !populated(&self.cgroup_parent) {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        // Session cgroups, then the scope; rmdir fails while one is populated.
        for entry in std::fs::read_dir(&self.cgroup_parent)
            .into_iter()
            .flatten()
            .flatten()
        {
            if entry.path().is_dir() {
                let _ = std::fs::remove_dir(entry.path());
            }
        }
        let _ = std::fs::remove_dir(&self.cgroup_parent);
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// A running `builder run`, killed on drop so a failed assert leaks no orch.
struct Orch(Process);

impl Drop for Orch {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Everything left in `pipe`.
fn read_all(pipe: Option<impl std::io::Read>) -> Vec<u8> {
    let mut bytes = Vec::new();
    if let Some(mut p) = pipe {
        p.read_to_end(&mut bytes).expect("pipe");
    }
    bytes
}

/// Wait up to `wait` for `done`, panicking with `what` past it.
fn wait_for(wait: Duration, what: &str, mut done: impl FnMut() -> bool) {
    let until = Instant::now() + wait;
    while !done() {
        assert!(Instant::now() < until, "{what}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// A claims task with the builder seat and an architect, both assigned.
fn open_task(root: &Path, bbp: &Path, repo: &Path) -> PathBuf {
    let role = |id: &str, order: u32| {
        format!(
            r#"{{"id":"{id}","turn_order":{order},"post":["Claim","Evidence"],"write":["Diff","Candidate","Source"],"read_artifacts":["Brief","Diff","Candidate","TestReport","Log","Source"],"read_messages":"All","verdict_at":[],"final_decision":false,"consultable":false}}"#
        )
    };
    std::fs::write(
        root.join("roster.json"),
        format!("[{},{}]", role("claude-builder", 0), role("architect", 1)),
    )
    .expect("roster");
    std::fs::write(root.join("brief.md"), "Toy claim.\n").expect("brief");
    let shipped = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../rusty_bbp_host/docs/proving/profiles.json"
    );
    let mut profiles: rusty_serde::Value =
        rusty_serde::json::from_str(&std::fs::read_to_string(shipped).expect("profiles"))
            .expect("profiles json");
    if let rusty_serde::Value::Map(m) = &mut profiles {
        m.retain(|(k, _)| k != "test_paths" && k != "erosion");
        m.push((
            "test_paths".into(),
            rusty_serde::Value::Seq(vec!["tests/**".into()]),
        ));
        m.push(("erosion".into(), "off".into()));
    }
    std::fs::write(
        root.join("profiles.json"),
        rusty_serde::json::to_string(&profiles).expect("encode"),
    )
    .expect("profiles");
    let dir = root.join("bbp");
    run(Command::new(bbp)
        .args(["open", "--mode", "claims", "--task", TASK, "--dir"])
        .arg(&dir)
        .arg("--repo")
        .arg(repo)
        .arg("--brief")
        .arg(root.join("brief.md"))
        .arg("--roster")
        .arg(root.join("roster.json"))
        .arg("--profiles")
        .arg(root.join("profiles.json")));
    for r in ["claude-builder", "architect"] {
        run(Command::new(bbp)
            .args([
                "assign",
                "--task",
                TASK,
                "--role",
                r,
                "--principal",
                r,
                "--vendor",
                "test",
            ])
            .arg("--dir")
            .arg(&dir));
    }
    dir
}

fn post_claim(bbp: &Path, dir: &Path, config: &Path) -> MsgId {
    let config =
        LensConfig::parse(&std::fs::read_to_string(config).expect("config")).expect("parse");
    let child = Child::spawn(bbp, dir, TASK, "architect").expect("architect");
    let mut s = Session::new("architect", child, &run_id().expect("run id"));
    s.refresh(&config).expect("refresh");
    let body = ClaimBody {
        text: CLAIM_TEXT.into(),
        domains: vec!["toy".into()],
        deps: vec![],
        answers: None,
        supersedes: None,
        stakes: Stakes::Low,
        reversible: true,
    };
    let ids = s
        .post("architect", &[Post::Claim { body, refs: vec![] }])
        .expect("claim posted");
    ids[0]
}

fn row(out: &Output) -> Value {
    let text = String::from_utf8_lossy(&out.stdout);
    Value::parse(text.trim()).unwrap_or_else(|e| {
        panic!(
            "row {text:?}: {e:?}; stderr {}",
            String::from_utf8_lossy(&out.stderr)
        )
    })
}

fn populated(cgroup_parent: &Path) -> bool {
    std::fs::read_dir(cgroup_parent)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .any(|e| {
            std::fs::read_to_string(e.path().join("cgroup.events"))
                .map(|ev| ev.contains("populated 1"))
                .unwrap_or(false)
        })
}

#[test]
#[ignore = "acceptance: needs the built binaries and a delegated cgroup"]
fn a_jailed_builder_stores_its_candidate_and_frees_its_slot() {
    let w = World::new("stored");
    let out = w.builder().output().expect("builder run");
    let row = row(&out);
    assert_eq!(row["state"], Value::from("stored"), "{row:?}");
    assert_eq!(row["tokens_in"].as_u64(), Some(11));
    let candidate = row["candidate"].as_u64().expect("candidate id");

    // The jail: no mux variable, no AF_UNIX socket (so no way to the mux
    // socket), and nothing writable outside the worktree.
    let probe = std::fs::read_to_string(w.worktree().join("probe.json")).expect("probe");
    let probe = Value::parse(&probe).expect("probe json");
    assert_eq!(probe["mux_env"], Value::Bool(false));
    for k in ["unix_socket", "write_outside"] {
        let v = probe[k].as_str().unwrap_or("");
        assert!(v.starts_with("refused"), "{k}: {v}");
    }

    // BBP stored it under the builder's seat for this claim.
    let card = w.card();
    assert_eq!(card.current.map(|a| a.0), Some(candidate));
    assert_eq!(card.spent, 1, "one candidate, one debit");

    // acp.log rendered the turn; the slot and the cgroup are free.
    let log = std::fs::read_to_string(w.run_dir().join("acp.log")).expect("acp.log");
    assert!(log.contains("fake builder wrote greeting.txt"), "{log}");
    assert!(
        w.slot_free(),
        "the slot is released once the cgroup is empty"
    );
    assert!(!populated(&w.cgroup_parent));
    println!("BBP_ACCEPTANCE PASS builder-stored");
}

#[test]
#[ignore = "acceptance: needs the built binaries and a delegated cgroup"]
fn a_mux_restart_aborts_the_turn_and_debits_nothing() {
    let mut w = World::new("restart");
    std::fs::write(w.fake.join("mode"), "hang").expect("mode");
    let mut orch = Orch(
        w.builder()
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("builder run"),
    );
    // The agent sits in its prompt once it has written `.prompted`.
    let prompted = w.worktree().join(".prompted");
    wait_for(
        Duration::from_secs(60),
        "the agent never took the prompt",
        || prompted.exists(),
    );
    w.kill_mux();
    w.start_mux();
    wait_for(Duration::from_secs(180), "builder run never ended", || {
        orch.0.try_wait().ok().flatten().is_some()
    });
    let stdout = read_all(orch.0.stdout.take());
    let stderr = read_all(orch.0.stderr.take());
    let out = Output {
        status: orch.0.wait().expect("status"),
        stdout,
        stderr,
    };
    let row = row(&out);
    assert_eq!(row["state"], Value::from("aborted"), "{row:?}");
    assert_eq!(row["candidate"], Value::Null);
    let card = w.card();
    assert_eq!((card.current, card.spent), (None, 0), "nothing debited");
    assert!(w.slot_free());
    assert!(!populated(&w.cgroup_parent));
    println!("BBP_ACCEPTANCE PASS builder-mux-restart");
}

#[test]
#[ignore = "acceptance: needs the built binaries and a delegated cgroup"]
fn the_holder_deadline_ends_the_session_with_orch_killed() {
    let w = World::new("deadline");
    std::fs::write(w.fake.join("mode"), "hang").expect("mode");
    // orch is killed before its 30 s wall, so only the holder's backstop
    // (wall + grace + margin from the session's start) can end it.
    let wall = Duration::from_secs(30);
    let config = LENS_CONFIG.replace("\"builder_wall_secs\": 600", "\"builder_wall_secs\": 30");
    std::fs::write(&w.config, config).expect("config");
    let started = Instant::now();
    let mut orch = Orch(w.builder().spawn().expect("builder run"));
    let prompted = w.worktree().join(".prompted");
    wait_for(
        Duration::from_secs(25),
        "the agent never took the prompt",
        || prompted.exists(),
    );
    orch.0.kill().expect("kill orch");
    orch.0.wait().expect("reap orch");
    assert!(
        started.elapsed() < wall,
        "orch reached its wall before the kill"
    );
    assert!(
        !w.slot_free(),
        "the holder keeps the slot while the agent runs"
    );

    let backstop = wall + orch_bbp::builder::GRACE + orch_bbp::builder::HOLDER_MARGIN;
    wait_for(
        backstop + Duration::from_secs(30),
        "the deadline never ended the agent",
        || !populated(&w.cgroup_parent) && w.slot_free(),
    );
    // Nothing but the holder's deadline could have ended it this late.
    let ended = started.elapsed();
    assert!(
        ended + Duration::from_secs(2) >= backstop,
        "ended after {ended:?}, before the holder's deadline {backstop:?}"
    );
    println!("BBP_ACCEPTANCE PASS builder-deadline");
}
